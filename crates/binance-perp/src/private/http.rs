use crate::private::types::{
    BinanceOrderStatus, BinanceOrderType, BinanceSide, BinanceTimeInForce,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
use chrono::{DateTime, Utc};
use ed25519_dalek::{pkcs8::DecodePrivateKey, Signer, SigningKey};
use rengine_interfaces::ExchangePrivateReader;
use rengine_types::{
    primitive::MarketType, Account, Action, BalanceKey, BookKey, Decimal, Instrument, Mapping,
    OpenOrder, OrderInfo, Side, Symbol, TimeInForce, Trade,
};
use rengine_utils::http::RequestExt;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Debug},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct BinancePerpPrivateReader {
    http: Client,
    base: Url,
    api_key: String,
    signer: Option<SigningKey>,
    account: Account,
    changes_tx: mpsc::Sender<Vec<Action>>,
    mapping: Mapping,
}

impl BinancePerpPrivateReader {
    pub fn new(
        api_key: String,
        secret_key: String,
        account: Account,
        changes_tx: mpsc::Sender<Vec<Action>>,
        mapping: Mapping,
    ) -> Result<Self> {
        let http = Client::builder().build()?;
        let base = Url::parse("https://fapi.binance.com")?;

        let signer = if secret_key.is_empty() {
            None
        } else {
            let pem_bytes = B64
                .decode(&secret_key)
                .context("decoding base64 PEM content")?;
            let pem =
                String::from_utf8(pem_bytes).context("converting PEM bytes to UTF-8 string")?;
            Some(SigningKey::from_pkcs8_pem(&pem).context("parsing Ed25519 PKCS#8 PEM")?)
        };

        Ok(Self {
            http,
            base,
            api_key,
            signer,
            account,
            changes_tx,
            mapping,
        })
    }

    fn timestamp_ms() -> u64 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        now.as_millis() as u64
    }

    fn sign_query(&self, query: &str) -> Result<String> {
        if let Some(signer) = &self.signer {
            let signature = signer.sign(query.as_bytes());
            Ok(B64.encode(signature.to_bytes()))
        } else {
            Err(anyhow!("Signer not initialized"))
        }
    }

    /// GET signed (`USER_DATA`) endpoint
    async fn get_signed<T: for<'de> Deserialize<'de> + fmt::Debug>(
        &self,
        path: &str,
        label: &str,
        mut query_pairs: Vec<(&str, String)>,
    ) -> Result<T> {
        query_pairs.push(("timestamp", Self::timestamp_ms().to_string()));

        // Build query string manually to sign in the exact order
        let mut q = String::new();
        for (i, (k, v)) in query_pairs.iter().enumerate() {
            if i > 0 {
                q.push('&');
            }
            q.push_str(k);
            q.push('=');
            q.push_str(
                serde_urlencoded::to_string(vec![("", v)])
                    .unwrap()
                    .trim_start_matches('='),
            );
        }
        let signature = self.sign_query(&q)?;
        let signature_param = serde_urlencoded::to_string([("signature", signature)])?;

        let mut url = self.base.join(path)?;
        url.set_query(Some(&format!("{q}&{signature_param}")));

        let res = self
            .http
            .get(url)
            .header("X-MBX-APIKEY", self.api_key.clone())
            .send_ok(label)
            .await
            .context("HTTP request failed")?;

        let status = res.status();

        if !status.is_success() {
            let text = res.text().await?;
            return Err(anyhow!("Binance error {status}: {text}"));
        }

        let parsed = res
            .json::<T>()
            .await
            .with_context(|| "Failed to parse Binance response")?;

        Ok(parsed)
    }

    pub async fn balances(&self) -> Result<Vec<BinanceBalance>> {
        self.get_signed("/fapi/v3/balance", "binance_perp_balances", vec![])
            .await
    }

    /// Returns /fapi/v1/openOrders; when `symbol` is None, Binance returns *all* open orders
    /// across symbols (weight cost is higher).
    pub async fn open_orders(&self, symbol: Option<&str>) -> Result<Vec<BinanceOrder>> {
        let mut q = Vec::new();
        if let Some(s) = symbol {
            q.push(("symbol", s.to_string()));
        }
        self.get_signed("/fapi/v1/openOrders", "binance_perp_open_orders", q)
            .await
    }

    /// Returns /fapi/v3/positionRisk (only symbols with a position or open orders)
    pub async fn positions(&self, symbol: Option<&str>) -> Result<Vec<BinancePosition>> {
        let mut q = Vec::new();
        if let Some(s) = symbol {
            q.push(("symbol", s.to_string()));
        }
        self.get_signed("/fapi/v3/positionRisk", "binance_perp_positions", q)
            .await
    }

    pub async fn trades(&self, symbol: &str, limit: Option<u32>) -> Result<Vec<BinanceUserTrade>> {
        let mut q = vec![("symbol", symbol.to_string())];

        if let Some(l) = limit {
            q.push(("limit", l.to_string()));
        }
        self.get_signed("/fapi/v1/userTrades", "binance_perp_trades", q)
            .await
    }
}

#[async_trait]
impl ExchangePrivateReader for BinancePerpPrivateReader {
    async fn fetch_open_orders(&self) -> Result<()> {
        let orders = self.open_orders(None).await?;
        let mut actions = Vec::new();

        for order in orders {
            let instrument = Instrument::from(order.symbol.clone());
            let side = match order.side {
                BinanceSide::Buy => Side::Bid,
                BinanceSide::Sell => Side::Ask,
            };
            let tif = match order.time_in_force {
                BinanceTimeInForce::Gtc => TimeInForce::GoodUntilCancelled,
                BinanceTimeInForce::Ioc | BinanceTimeInForce::Fok => TimeInForce::Unknown,
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
        let balances = self.balances().await?;
        let mut result = Vec::new();

        for bal in balances {
            let asset_symbol = Symbol::from(bal.asset.as_str());
            let symbol = match self.mapping.map_symbol(&self.account.venue, &asset_symbol) {
                Ok(symbol) => symbol.clone(),
                Err(_) => continue,
            };

            let key = BalanceKey {
                account: self.account.clone(),
                symbol,
            };
            result.push((key, bal.available_balance));
        }
        Ok(result)
    }

    async fn fetch_trades(&self) -> Result<Vec<(BookKey, Trade)>> {
        let mut all_trades = Vec::new();
        if let Some(instruments) = self.mapping.instruments(&self.account.venue) {
            for (instrument, details) in instruments {
                let symbol = format!("{}{}", details.base, details.quote).to_uppercase();

                if let Ok(trades) = self.trades(&symbol, Some(50)).await {
                    for trade in trades {
                        let side = match trade.side {
                            BinanceSide::Buy => Side::Bid,
                            BinanceSide::Sell => Side::Ask,
                        };

                        let t = Trade {
                            emitted_at: trade.time.into(),
                            received_at: rengine_types::Timestamp::now(),
                            order_id: trade.order_id as i64,
                            trade_id: trade.id as i64,
                            account: self.account.clone(),
                            base: details.base.clone(),
                            quote: details.quote.clone(),
                            side,
                            market_type: MarketType::Perp,
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
                    }
                }
            }
        }
        Ok(all_trades)
    }

    async fn fetch_positions(&self) -> Result<()> {
        let positions = self.positions(None).await?;
        let mut actions = Vec::new();

        for pos in positions {
            if pos.position_amt.is_zero() {
                continue;
            }
            let instrument = Instrument::from(pos.symbol.clone());
            let book_key = BookKey {
                account: self.account.clone(),
                instrument,
            };
            actions.push(Action::SetPerpPosition(book_key, pos.position_amt));
        }

        if !actions.is_empty() {
            self.changes_tx
                .send(actions)
                .await
                .map_err(|e| anyhow!("Failed to send changes: {}", e))?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceBalance {
    pub account_alias: String,
    pub asset: String,
    pub balance: Decimal,
    pub cross_wallet_balance: Decimal,
    pub cross_un_pnl: Decimal,
    pub available_balance: Decimal,
    pub max_withdraw_amount: Decimal,
    pub margin_available: bool,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub update_time: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceOrder {
    pub avg_price: Decimal,
    pub client_order_id: String,
    pub cum_quote: Decimal,
    pub executed_qty: Decimal,
    pub order_id: i64,
    pub orig_qty: Decimal,
    pub orig_type: String,
    pub price: Decimal,
    pub reduce_only: bool,
    pub side: BinanceSide,
    pub position_side: String,
    pub status: BinanceOrderStatus,
    pub stop_price: Option<Decimal>,
    pub close_position: bool,
    pub symbol: String,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub time: DateTime<Utc>,
    pub time_in_force: BinanceTimeInForce,
    #[serde(rename = "type")]
    pub order_type: BinanceOrderType,
    pub activate_price: Option<Decimal>,
    pub price_rate: Option<Decimal>,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub update_time: DateTime<Utc>,
    pub working_type: String,
    pub price_protect: bool,
    pub price_match: String,
    pub self_trade_prevention_mode: String,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub good_till_date: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinancePosition {
    pub symbol: String,
    pub position_side: String,
    pub position_amt: Decimal,
    pub entry_price: Decimal,
    pub break_even_price: Decimal,
    pub mark_price: Decimal,
    pub un_realized_profit: Decimal,
    pub liquidation_price: Decimal,
    pub isolated_margin: Decimal,
    pub notional: Decimal,
    pub margin_asset: String,
    pub isolated_wallet: Decimal,
    pub initial_margin: Decimal,
    pub maint_margin: Decimal,
    pub position_initial_margin: Decimal,
    pub open_order_initial_margin: Decimal,
    pub adl: i32,
    // pub bid_notional: Decimal,
    // pub ask_notional: Decimal,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub update_time: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceUserTrade {
    pub symbol: String,
    pub id: u64,
    pub order_id: u64,
    // pub pair: String,
    pub side: BinanceSide,
    pub price: Decimal,
    pub qty: Decimal,
    // pub realized_pnl: Decimal,
    // pub margin_asset: String,
    // pub base_qty: Decimal,
    pub commission: Decimal,
    pub commission_asset: String,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub time: DateTime<Utc>,
    // pub position_side: BinancePositionSide,
    // pub buyer: bool,
    pub maker: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rengine_types::{Mapping, MappingInner};
    use std::env;

    #[tokio::test]
    #[ignore]
    async fn test_binance_perp_http_reader() {
        // Needs valid API Key for real test, but this is ignored.
        let api_key = env::var("BINANCE_PERP_API_KEY").unwrap_or_default();
        let secret_key = env::var("BINANCE_PERP_SECRET_KEY").unwrap_or_default();

        if api_key.is_empty() || secret_key.is_empty() {
            println!("Skipping test due to missing API keys");
            return;
        }

        let account = Account {
            account_id: "test".into(),
            venue: "binance_perp".into(),
            market_type: MarketType::Perp,
        };
        let (changes_tx, _rx) = mpsc::channel(100);
        let mapping = Mapping::new(MappingInner::default());

        let reader =
            BinancePerpPrivateReader::new(api_key, secret_key, account, changes_tx, mapping)
                .unwrap();

        // Test balances
        let balances = reader.balances().await;
        println!("Balances: {:?}", balances);

        // Test open_orders
        let open_orders = reader.open_orders(None).await;
        println!("Open Orders: {:?}", open_orders);

        // Test positions
        let positions = reader.positions(None).await;
        println!("Positions: {:?}", positions);

        // Test trades
        let trades = reader.trades("BTCUSDT", Some(5)).await;
        println!("Trades: {:?}", trades);
    }
}
