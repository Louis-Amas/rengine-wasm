use crate::{
    execution::types::{BinanceOrderType, BinanceSide, BinanceTimeInForce},
    private::signer_from_pem_b64,
    public::http::HttpClient,
};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
use chrono::{serde::ts_milliseconds, DateTime, Utc};
use ed25519_dalek::{ed25519::signature::SignerMut, SigningKey};
use rengine_interfaces::ExchangePrivateReader;
use rengine_non_wasm_types::ChangesTx;
use rengine_types::{
    Account, Action, BalanceKey, BookKey, Decimal, Instrument, Mapping, OpenOrder, OrderInfo, Side,
    Symbol, TimeInForce, Trade,
};
use rengine_utils::http::RequestExt;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    env,
};

const REST_URL: &str = "https://api.binance.com";

#[derive(Clone)]
pub struct BinanceSpotPrivateReader {
    account: Account,
    api_key: String,
    http_client: HttpClient,
    changes_tx: ChangesTx,
    symbol_map: HashMap<String, Instrument>,
    signer: Option<SigningKey>,
    mapping: Mapping,
}

impl BinanceSpotPrivateReader {
    pub fn new(
        account: Account,
        api_key: String,
        secret_key: String,
        changes_tx: ChangesTx,
        mapping: Mapping,
    ) -> Result<Self> {
        let mut symbol_map = HashMap::new();
        if let Some(instruments) = mapping.instruments(&account.venue) {
            for (instrument, details) in instruments {
                // Binance Spot symbol format: BASEQUOTE (e.g. BTCUSDT)
                let symbol = format!("{}{}", details.base, details.quote).to_uppercase();
                symbol_map.insert(symbol, instrument.clone());
            }
        }

        let api_key = api_key.trim().to_string();
        let secret_key = secret_key.trim().to_string();

        let signer = if secret_key.is_empty() {
            None
        } else {
            Some(signer_from_pem_b64(&secret_key)?)
        };

        let base_url = env::var("BINANCE_SPOT_API_URL").unwrap_or_else(|_| REST_URL.to_string());

        Ok(Self {
            account,
            api_key,
            http_client: HttpClient::new(base_url),
            changes_tx,
            symbol_map,
            signer,
            mapping,
        })
    }

    async fn get_signed(
        &self,
        path: &str,
        label: &str,
        mut params: BTreeMap<String, String>,
    ) -> Result<Vec<u8>> {
        let ts = Utc::now().timestamp_millis().to_string();
        params.insert("timestamp".into(), ts);

        let query_str = serde_urlencoded::to_string(&params)?;

        let signature = if let Some(mut signer) = self.signer.clone() {
            B64.encode(signer.sign(query_str.as_bytes()).to_bytes())
        } else {
            bail!("Signer not initialized");
        };

        params.insert("signature".into(), signature);

        // Use the shared HttpClient's inner reqwest client
        let resp = self
            .http_client
            .get(path)
            .query(&params)
            .header("X-MBX-APIKEY", &self.api_key)
            .send_ok(label)
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            return Err(anyhow!("Binance API error: {}", text));
        }

        Ok(resp.bytes().await?.to_vec())
    }

    async fn get_orders(&self) -> Result<Vec<u8>> {
        self.get_signed(
            "/api/v3/openOrders",
            "binance_spot_open_orders",
            BTreeMap::new(),
        )
        .await
    }

    async fn get_balances(&self) -> Result<Vec<u8>> {
        let mut params = BTreeMap::new();
        params.insert("omitZeroBalances".into(), "true".into());
        self.get_signed("/api/v3/account", "binance_spot_balances", params)
            .await
    }
}

#[async_trait]
impl ExchangePrivateReader for BinanceSpotPrivateReader {
    async fn fetch_open_orders(&self) -> Result<()> {
        let data = self.get_orders().await?;
        let binance_orders: Vec<BinanceOrder> = serde_json::from_slice(&data)?;

        let mut actions = Vec::new();
        for order in binance_orders {
            let instrument = Instrument::from(order.symbol.clone());
            let side = match order.side {
                BinanceSide::Buy => Side::Bid,
                BinanceSide::Sell => Side::Ask,
            };
            let tif = match order.time_in_force {
                BinanceTimeInForce::Gtc => TimeInForce::GoodUntilCancelled,
                BinanceTimeInForce::Ioc | BinanceTimeInForce::Fok => TimeInForce::Unknown, // Map to Unknown as per previous decision
                BinanceTimeInForce::Gtx => TimeInForce::PostOnly,
            };

            let order_info = OrderInfo::new(side, order.price, order.orig_qty, tif)
                .with_client_order_id(order.client_order_id.clone().into());

            let open_order = OpenOrder {
                info: order_info,
                original_size: order.orig_qty,
                is_snapshot: true,
            };

            let book_key = BookKey {
                account: self.account.clone(),
                instrument,
            };

            actions.push(Action::SetOpenOrder(
                book_key,
                order.order_id.to_string(),
                open_order,
            ));
        }

        if !actions.is_empty() {
            self.changes_tx
                .send(actions)
                .await
                .map_err(|e| anyhow!("Failed to send changes: {}", e))?;
        }
        Ok(())
    }

    async fn fetch_balances(&self) -> Result<Vec<(BalanceKey, Decimal)>> {
        let data = self.get_balances().await?;
        let account_info: BinanceAccountPayload = serde_json::from_slice(&data)?;

        let mut balances = Vec::new();
        for bal in account_info.balances {
            let asset_symbol = Symbol::from(bal.asset.as_str());
            let symbol = match self.mapping.map_symbol(&self.account.venue, &asset_symbol) {
                Ok(symbol) => symbol.clone(),
                Err(err) => {
                    tracing::error!(?err, asset = %bal.asset, "Failed to map symbol");
                    continue;
                }
            };

            let balance_key = BalanceKey {
                account: self.account.clone(),
                symbol,
            };
            // Use free (available) balance
            balances.push((balance_key, bal.free));
        }
        Ok(balances)
    }

    async fn fetch_trades(&self) -> Result<Vec<(BookKey, Trade)>> {
        let mut all_trades = Vec::new();

        // Iterate and fetch for each symbol
        // We can't use `get_trades` helper easily if it returns bytes of merged JSON.
        // So I'll implement the loop here.

        // Limit concurrency to avoid rate limits?
        // For now, sequential or small chunks.
        for (binance_symbol, instrument) in &self.symbol_map {
            let mut params = BTreeMap::new();
            params.insert("symbol".into(), binance_symbol.clone());
            params.insert("limit".into(), "100".into());

            // Ignore errors for individual symbols to avoid failing the whole sync?
            if let Ok(data) = self
                .get_signed("/api/v3/myTrades", "binance_spot_trades", params)
                .await
            {
                if let Ok(trades) = serde_json::from_slice::<Vec<BinanceTrade>>(&data) {
                    for trade in trades {
                        let side = if trade.is_buyer { Side::Bid } else { Side::Ask };

                        if let Ok(details) =
                            self.mapping.map_instrument(&self.account.venue, instrument)
                        {
                            let t = Trade {
                                emitted_at: trade.time.into(),
                                received_at: rengine_types::Timestamp::now(),
                                order_id: trade.order_id as i64,
                                trade_id: trade.id as i64,
                                account: self.account.clone(),
                                base: details.base.clone(),
                                quote: details.quote.clone(),
                                side,
                                market_type: rengine_types::MarketType::Spot,
                                price: trade.price,
                                size: trade.qty,
                                fee: trade.commission,
                                fee_symbol: trade.commission_asset.into(),
                            };

                            let book_key = BookKey {
                                account: self.account.clone(),
                                instrument: instrument.clone(),
                            };
                            all_trades.push((book_key, t));
                        } else {
                            tracing::error!("Failed to map instrument {}", instrument);
                        }
                    }
                }
            }
        }

        Ok(all_trades)
    }

    async fn fetch_positions(&self) -> Result<()> {
        Ok(())
    }
}

// Helper Structs

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct BinanceOrder {
    symbol: String,
    order_id: u64,
    client_order_id: String,
    price: Decimal,
    orig_qty: Decimal,
    executed_qty: Decimal,
    status: String,
    time_in_force: BinanceTimeInForce,
    r#type: BinanceOrderType,
    side: BinanceSide,
    #[serde(with = "ts_milliseconds")]
    time: DateTime<Utc>,
    #[serde(with = "ts_milliseconds")]
    update_time: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BinanceBalanceEntry {
    asset: String,
    free: Decimal,
    locked: Decimal,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinanceAccountPayload {
    balances: Vec<BinanceBalanceEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct BinanceTrade {
    symbol: String,
    id: u64,
    order_id: u64,
    price: Decimal,
    qty: Decimal,
    commission: Decimal,
    commission_asset: String,
    #[serde(with = "ts_milliseconds")]
    time: DateTime<Utc>,
    is_buyer: bool,
    is_maker: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rengine_types::MappingInner;
    use tokio::sync::mpsc;

    #[tokio::test]
    #[ignore]
    async fn test_binance_spot_private_reader() {
        // Setup fake account and mapping
        let account = Account {
            account_id: "test".into(),
            venue: "binance-spot".into(),
            market_type: rengine_types::MarketType::Spot,
        };

        let toml_str = r#"
[instrument_mapping."binance-spot".BTCUSDC]
base = "btc"
quote = "usdc"
marketType = "spot"

[token_mapping]
"#;
        let mapping_inner: MappingInner = toml::from_str(toml_str).unwrap();
        let mapping = Mapping::new(mapping_inner);

        let (changes_tx, mut changes_rx) = mpsc::channel(100);

        // Needs valid API Key for real test, but this is ignored.
        let api_key = env::var("BINANCE_API_KEY").unwrap_or_default();
        let secret_key = env::var("BINANCE_SECRET_KEY").unwrap_or_default();

        if api_key.is_empty() {
            println!("Skipping test due to missing API keys");
            return;
        }

        let reader =
            BinanceSpotPrivateReader::new(account, api_key, secret_key, changes_tx, mapping)
                .unwrap();

        // Test fetch_balances
        let balances = reader.fetch_balances().await;
        println!("Balances: {:?}", balances);

        // Test fetch_open_orders
        let _ = reader.fetch_open_orders().await;

        // Read from channel
        while let Ok(actions) = changes_rx.try_recv() {
            println!("Open Orders actions: {:?}", actions);
        }

        // Test fetch_trades
        let trades = reader.fetch_trades().await;
        println!("Trades result: {:?}", trades);
    }
}
