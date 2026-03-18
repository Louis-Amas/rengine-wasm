use crate::{http::HttpClient, types::Direction};
use alloy::primitives::Address;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use rengine_interfaces::ExchangePrivateReader;
use rengine_non_wasm_types::{send_changes, ChangesTx};
use rengine_types::{
    Account, Action, BalanceKey, BookKey, Mapping, MarketType, OpenOrder, OrderInfo, TimeInForce,
    Trade,
};
use rust_decimal::Decimal;
use tokio::sync::broadcast;
use tracing::{error, warn};

pub struct HyperLiquidPrivateReader {
    client: HttpClient,
    account: Account,
    changes_tx: ChangesTx,
    account_address: Address,
    mapping: Mapping,
}

impl HyperLiquidPrivateReader {
    pub(crate) fn new(
        account: Account,
        changes_tx: ChangesTx,
        account_address: Address,
        instrument_mapping: Mapping,
    ) -> Self {
        Self {
            client: HttpClient::default(),
            account,
            changes_tx,
            account_address,
            mapping: instrument_mapping,
        }
    }

    pub(crate) async fn run_on_reconnect(
        self,
        mut reconnect_rx: broadcast::Receiver<()>,
    ) -> Result<()> {
        while reconnect_rx.recv().await.is_ok() {
            let _ = self.sync_state(&self.changes_tx).await;
        }
        Ok(())
    }
}

#[async_trait]
impl ExchangePrivateReader for HyperLiquidPrivateReader {
    async fn fetch_open_orders(&self) -> Result<()> {
        let results = self.client.open_orders(self.account_address).await?;

        let actions: Vec<_> = results
            .into_iter()
            .map(|o| {
                let key = BookKey {
                    account: self.account.clone(),
                    instrument: o.coin.clone(),
                };
                let size = o.sz;
                let mut info =
                    OrderInfo::new(o.side.into(), o.limit_px, o.sz, TimeInForce::Unknown);
                if let Some(cloid) = o.cloid {
                    info = info.with_client_order_id(cloid);
                }

                let open_order = OpenOrder {
                    info,
                    original_size: size,
                    is_snapshot: false,
                };
                Action::SetOpenOrder(key, o.oid.to_string(), open_order)
            })
            .collect();

        send_changes(&self.changes_tx, actions);

        Ok(())
    }

    async fn fetch_balances(&self) -> Result<Vec<(BalanceKey, Decimal)>> {
        if self.account.market_type == MarketType::Spot {
            let result = self
                .client
                .user_token_balances(self.account_address)
                .await?;

            let balances: Vec<_> = result
                .balances
                .into_iter()
                .filter_map(|balance| {
                    let symbol = match self.mapping.map_symbol(&self.account.venue, &balance.coin) {
                        Ok(symbol) => symbol,
                        Err(err) => {
                            error!(?err);
                            return None;
                        }
                    };

                    let key = BalanceKey {
                        symbol: symbol.clone(),
                        account: self.account.clone(),
                    };
                    Some((key, balance.total - balance.hold))
                })
                .collect();

            Ok(balances)
        } else {
            let result = self.client.user_state(self.account_address).await?;

            let usdc = "USDC".into();
            let symbol = self
                .mapping
                .map_symbol(&self.account.venue, &usdc)
                .cloned()
                .unwrap_or_else(|_| "usdc".into());

            let key = BalanceKey {
                symbol,
                account: self.account.clone(),
            };

            Ok(vec![(key, result.withdrawable)])
        }
    }

    async fn fetch_trades(&self) -> Result<Vec<(BookKey, Trade)>> {
        let trades = self.client.trades(self.account_address).await?;

        let time = Utc::now();
        let trades: Vec<_> = trades
            .into_iter()
            .filter_map(|trade| {
                let Some(instrument) = self
                    .mapping
                    .map_instrument(&self.account.venue, &trade.coin)
                    .ok()
                else {
                    warn!(?self.account, ?trade.coin, "missing mapping for");
                    return None;
                };

                let book_key = BookKey {
                    account: self.account.clone(),
                    instrument: trade.coin.clone(),
                };

                if instrument.market_type == self.account.market_type {
                    if self.account.market_type == MarketType::Spot {
                        matches!(trade.dir, Direction::Buy | Direction::Sell).then(|| {
                            (
                                book_key,
                                trade.to_engine_trade(&instrument, self.account.clone(), time),
                            )
                        })
                    } else {
                        (!matches!(trade.dir, Direction::Buy | Direction::Sell)).then(|| {
                            (
                                book_key,
                                trade.to_engine_trade(&instrument, self.account.clone(), time),
                            )
                        })
                    }
                } else {
                    None
                }
            })
            .collect();

        Ok(trades)
    }
    async fn fetch_positions(&self) -> Result<()> {
        if self.account.market_type == MarketType::Spot {
            return Ok(());
        }

        let result = self.client.user_state(self.account_address).await?;

        let actions: Vec<_> = result
            .asset_positions
            .into_iter()
            .map(|pos| {
                let key = BookKey {
                    account: self.account.clone(),
                    instrument: pos.position.coin.clone(),
                };
                Action::SetPerpPosition(key, pos.position.szi)
            })
            .collect();

        send_changes(&self.changes_tx, actions);

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::private::http::HyperLiquidPrivateReader;
    use alloy::primitives::address;
    use rengine_interfaces::ExchangePrivateReader;
    use rengine_types::{Mapping, MappingInner};
    use tokio::sync::mpsc::channel;

    #[tokio::test]
    #[ignore]
    async fn test_fetch_trades() {
        let (tx, _rx) = channel(16);

        let account = "hyperliquid|perp|account".parse().unwrap();
        let addr = address!("0xa6F1eF0733FC462627f280053040f5f47D7fA0c6");
        let toml_str = r#"
[instrument_mapping.hyperliquid.ETH]
base = "eth"
quote = "usd"
marketType = "perp"

[instrument_mapping.hyperliquid."@151"]
base = "eth"
quote = "usd"
marketType = "spot"

[token_mapping]
"#;

        let mapping: MappingInner =
            toml::from_str(toml_str).expect("should deserialize InstrumentMapping from TOML");
        let mapping = Mapping::new(mapping);

        let client = HyperLiquidPrivateReader::new(account, tx, addr, mapping);

        let trades = client.fetch_trades().await.unwrap();

        println!("{trades:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn test_fetch_balances() {
        let (tx, _rx) = channel(16);

        let account = "hyperliquid|perp|account".parse().unwrap();
        let addr = address!("0xa6F1eF0733FC462627f280053040f5f47D7fA0c6");
        let toml_str = r#"
[instrument_mapping.hyperliquid.ETH]
base = "eth"
quote = "usd"
marketType = "perp"

[instrument_mapping.hyperliquid."@151"]
base = "eth"
quote = "usd"
marketType = "spot"

[token_mapping]
"#;

        let mapping: MappingInner =
            toml::from_str(toml_str).expect("should deserialize InstrumentMapping from TOML");
        let mapping = Mapping::new(mapping);

        let client = HyperLiquidPrivateReader::new(account, tx, addr, mapping);

        let balances = client.fetch_balances().await.unwrap();

        println!("{balances:?}");
    }
}
