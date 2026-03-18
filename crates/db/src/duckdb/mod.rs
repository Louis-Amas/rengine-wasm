mod evm_logs;
mod multicalls;
mod strategies;
mod trades;
mod transformers;
mod utils;

use self::{
    evm_logs::{handle_add_evm_logs, handle_list_evm_logs, handle_remove_evm_logs},
    multicalls::{handle_add_multicall, handle_list_multicalls, handle_remove_multicall},
    strategies::{handle_add_strategy, handle_list_strategies, handle_toggle_strategy},
    trades::{handle_add_trades, handle_list_trades, handle_load_exposures},
    transformers::{handle_add_transformer, handle_list_transformers, handle_toggle_transformer},
};
use anyhow::Result;
use duckdb::Connection;
use rengine_interfaces::db::{
    EvmLogsRepository, MultiCallRepository, StrategyRepository, TradesRepository,
    TransformerRepository,
};
use rengine_types::{
    db::{EvmLogsDb, Exposure, MultiCallDb, StrategyDb, TradeDb, TransformerDb},
    Account, MultiCallId, Venue,
};
use tokio::{
    sync::{
        mpsc::{self, unbounded_channel},
        oneshot,
    },
    task::JoinHandle,
};

enum DuckDbMessage {
    AddStrategy {
        strategy: StrategyDb,
        resp: oneshot::Sender<Result<()>>,
    },
    AddTransformer {
        transformer: TransformerDb,
        resp: oneshot::Sender<Result<()>>,
    },
    AddMultiCall {
        multicall: MultiCallDb,
        resp: oneshot::Sender<Result<()>>,
    },
    RemoveMultiCall {
        venue: Venue,
        name: MultiCallId,
        resp: oneshot::Sender<Result<()>>,
    },
    ListMulticall {
        venue: Venue,
        resp: oneshot::Sender<Result<Vec<MultiCallDb>>>,
    },
    ToggleStrategy {
        name: String,
        enabled: bool,
        resp: oneshot::Sender<Result<()>>,
    },
    ToggleTransformer {
        name: String,
        enabled: bool,
        resp: oneshot::Sender<Result<()>>,
    },
    ListStrategies {
        resp: oneshot::Sender<Result<Vec<StrategyDb>>>,
    },
    ListTransformers {
        resp: oneshot::Sender<Result<Vec<TransformerDb>>>,
    },
    AddTrades {
        trades: Vec<TradeDb>,
        resp: oneshot::Sender<Result<()>>,
    },
    ListTrades {
        account: Account,
        resp: oneshot::Sender<Result<Vec<TradeDb>>>,
    },
    LoadExposures {
        account: Account,
        resp: oneshot::Sender<Result<Vec<Exposure>>>,
    },
    AddEvmLogs {
        evm_logs: EvmLogsDb,
        resp: oneshot::Sender<Result<()>>,
    },
    RemoveEvmLogs {
        venue: Venue,
        name: String,
        resp: oneshot::Sender<Result<()>>,
    },
    ListEvmLogs {
        venue: Venue,
        resp: oneshot::Sender<Result<Vec<EvmLogsDb>>>,
    },
}

async fn run_duckdb_actor(
    mut rx: mpsc::UnboundedReceiver<DuckDbMessage>,
    db_path: &str,
) -> Result<JoinHandle<()>> {
    let mut conn = Connection::open(db_path)?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS strategy (
            strategy_name TEXT PRIMARY KEY,
            wasm BLOB NOT NULL,
            enabled BOOLEAN NOT NULL
        );
        "#,
        [],
    )?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS transformer (
            transformer_name TEXT PRIMARY KEY,
            wasm BLOB NOT NULL,
            enabled BOOLEAN NOT NULL
        );
        "#,
        [],
    )?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS multicall (
            venue TEXT NOT NULL,
            name TEXT NOT NULL,
            wasm BLOB NOT NULL,
            PRIMARY KEY (venue, name)
        );
        "#,
        [],
    )?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS trade (
            received_at   TIMESTAMP    NOT NULL,
            emitted_at    TIMESTAMP    NOT NULL,
            order_id      BIGINT       NOT NULL,
            trade_id      BIGINT       NOT NULL UNIQUE,
            account       TEXT         NOT NULL,
            base          TEXT         NOT NULL,
            quote         TEXT         NOT NULL,
            side          TEXT         NOT NULL,
            market_type   TEXT         NOT NULL,
            price         DECIMAL(28,18) NOT NULL,
            size          DECIMAL(28,18) NOT NULL,
            fee           DECIMAL(28,18) NOT NULL,
            fee_symbol    TEXT         NOT NULL
        );
        "#,
        [],
    )?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS evm_logs (
            venue TEXT NOT NULL,
            name TEXT NOT NULL,
            wasm BLOB NOT NULL,
            PRIMARY KEY (venue, name)
        );
        "#,
        [],
    )?;

    let handle = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                DuckDbMessage::AddStrategy { strategy, resp } => {
                    handle_add_strategy(&conn, strategy, resp);
                }
                DuckDbMessage::AddTransformer { transformer, resp } => {
                    handle_add_transformer(&conn, transformer, resp);
                }
                DuckDbMessage::AddMultiCall { multicall, resp } => {
                    handle_add_multicall(&conn, multicall, resp);
                }
                DuckDbMessage::RemoveMultiCall { venue, name, resp } => {
                    handle_remove_multicall(&conn, venue, name, resp);
                }
                DuckDbMessage::ListMulticall { venue, resp } => {
                    handle_list_multicalls(&conn, venue, resp);
                }
                DuckDbMessage::ToggleStrategy {
                    name,
                    enabled,
                    resp,
                } => {
                    handle_toggle_strategy(&conn, name, enabled, resp);
                }
                DuckDbMessage::ToggleTransformer {
                    name,
                    enabled,
                    resp,
                } => {
                    handle_toggle_transformer(&conn, name, enabled, resp);
                }

                DuckDbMessage::ListStrategies { resp } => {
                    handle_list_strategies(&conn, resp);
                }
                DuckDbMessage::ListTransformers { resp } => {
                    handle_list_transformers(&conn, resp);
                }
                DuckDbMessage::AddTrades { trades, resp } => {
                    handle_add_trades(&mut conn, trades, resp);
                }

                DuckDbMessage::ListTrades { account, resp } => {
                    handle_list_trades(&conn, account, resp);
                }
                DuckDbMessage::LoadExposures { account, resp } => {
                    handle_load_exposures(&conn, account, resp);
                }
                DuckDbMessage::AddEvmLogs { evm_logs, resp } => {
                    handle_add_evm_logs(&conn, evm_logs, resp);
                }
                DuckDbMessage::RemoveEvmLogs { venue, name, resp } => {
                    handle_remove_evm_logs(&conn, venue, name, resp);
                }
                DuckDbMessage::ListEvmLogs { venue, resp } => {
                    handle_list_evm_logs(&conn, venue, resp);
                }
            }
        }
    });

    Ok(handle)
}

pub struct DuckDb {
    tx: mpsc::UnboundedSender<DuckDbMessage>,
    handle: JoinHandle<()>,
}

impl DuckDb {
    pub async fn new(db_path: &str) -> Result<Self> {
        let (tx, rx) = unbounded_channel();
        let handle = run_duckdb_actor(rx, db_path).await?;

        Ok(Self { tx, handle })
    }

    pub fn stop(&self) {
        self.handle.abort();
    }
}

#[async_trait::async_trait]
impl TradesRepository for DuckDb {
    async fn record_trades(&self, trades: Vec<TradeDb>) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::AddTrades {
            trades,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }

    async fn list_trades(&self, account: Account) -> Result<Vec<TradeDb>> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::ListTrades {
            account,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }

    async fn load_exposures(&self, account: Account) -> Result<Vec<Exposure>> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::LoadExposures {
            account,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }
}

#[async_trait::async_trait]
impl StrategyRepository for DuckDb {
    async fn add_strategy(&self, strategy: StrategyDb) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::AddStrategy {
            strategy,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }

    async fn set_enable(&self, name: &str, enabled: bool) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::ToggleStrategy {
            name: name.to_string(),
            enabled,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }

    async fn list_strategies(&self) -> Result<Vec<StrategyDb>> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(DuckDbMessage::ListStrategies { resp: resp_tx })?;
        resp_rx.await?
    }
}

#[async_trait::async_trait]
impl TransformerRepository for DuckDb {
    async fn add_transformer(&self, transformer: TransformerDb) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::AddTransformer {
            transformer,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }

    async fn set_transformer_enable(&self, name: &str, enabled: bool) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::ToggleTransformer {
            name: name.to_string(),
            enabled,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }

    async fn list_transformers(&self) -> Result<Vec<TransformerDb>> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(DuckDbMessage::ListTransformers { resp: resp_tx })?;
        resp_rx.await?
    }
}

#[async_trait::async_trait]
impl MultiCallRepository for DuckDb {
    async fn add_multicall_reader(&self, multicall: MultiCallDb) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::AddMultiCall {
            multicall,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }
    async fn remove_multicall_reader(&self, venue: Venue, name: MultiCallId) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::RemoveMultiCall {
            venue,
            name,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }

    async fn list_multicall(&self, venue: Venue) -> Result<Vec<MultiCallDb>> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::ListMulticall {
            venue,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }
}

#[async_trait::async_trait]
impl EvmLogsRepository for DuckDb {
    async fn add_evm_logs(&self, evm_logs: EvmLogsDb) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::AddEvmLogs {
            evm_logs,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }

    async fn remove_evm_logs(&self, venue: Venue, name: String) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::RemoveEvmLogs {
            venue,
            name,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }

    async fn list_evm_logs(&self, venue: Venue) -> Result<Vec<EvmLogsDb>> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(DuckDbMessage::ListEvmLogs {
            venue,
            resp: resp_tx,
        })?;
        resp_rx.await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use rengine_interfaces::db::StrategyRepository;
    use rengine_types::{db::StrategyDb, Side};
    use rust_decimal_macros::dec;

    #[tokio::test]
    async fn test_duckdb_actor() -> Result<()> {
        let db = DuckDb::new(":memory:").await?;

        let strategy = StrategyDb {
            strategy_name: "test_strategy".to_string(),
            wasm: vec![0x00, 0x61, 0x73, 0x6d],
            enabled: true,
        };

        db.add_strategy(strategy.clone()).await?;

        let strategies = db.list_strategies().await?;
        assert_eq!(strategies.len(), 1);

        let s = &strategies[0];
        assert_eq!(s.strategy_name, strategy.strategy_name);
        assert_eq!(s.wasm, strategy.wasm);
        assert!(s.enabled);

        db.set_enable(&strategy.strategy_name, false).await?;

        let strategies = db.list_strategies().await?;
        assert_eq!(strategies.len(), 1);
        assert!(!strategies[0].enabled);

        let account: Account = "hyperliquid|spot|account".parse().unwrap();
        let trade = TradeDb {
            received_at: Utc::now(),
            emitted_at: Utc::now(),
            order_id: 123,
            trade_id: 123,
            account: account.to_string(),
            base: "BTC".into(),
            quote: "USD".into(),
            side: Side::Bid.to_string(),
            market_type: "spot".into(),
            price: dec!(45000),
            size: dec!(0.1),
            fee: dec!(0.0005),
            fee_symbol: "BTC".into(),
        };

        db.record_trades(vec![trade]).await.unwrap();

        let trades = db.list_trades(account.clone()).await.unwrap();
        assert_eq!(trades.len(), 1);

        let exposure = db.load_exposures(account).await.unwrap();
        println!("{exposure:?}");

        db.stop();

        Ok(())
    }

    #[tokio::test]
    async fn test_exposure_multiple_trades() {
        let db = DuckDb::new(":memory:").await.unwrap();
        let account: Account = "hyperliquid|spot|account".parse().unwrap();

        // Trade 1: Buy 0.1 BTC @ 45,000
        let t0 = Utc::now();

        // Trade 1: Buy 0.1 BTC @ 45,000
        let trade1 = TradeDb {
            received_at: t0,
            emitted_at: t0,
            order_id: 1,
            trade_id: 1,
            account: account.to_string(),
            base: "BTC".into(),
            quote: "USD".into(),
            side: Side::Bid.to_string(),
            market_type: "spot".into(),
            price: dec!(45000),
            size: dec!(0.1),
            fee: dec!(0.0005),
            fee_symbol: "BTC".into(),
        };

        // Trade 2: Buy 0.2 BTC @ 40,000
        let trade2 = TradeDb {
            received_at: t0 + chrono::Duration::hours(1),
            emitted_at: t0 + chrono::Duration::hours(1),
            order_id: 2,
            trade_id: 2,
            account: account.to_string(),
            base: "BTC".into(),
            quote: "USD".into(),
            side: Side::Bid.to_string(),
            market_type: "spot".into(),
            price: dec!(40000),
            size: dec!(0.2),
            fee: dec!(0.0010),
            fee_symbol: "BTC".into(),
        };

        // Trade 3: Sell 0.15 BTC @ 50,000
        let trade3 = TradeDb {
            received_at: t0 + chrono::Duration::hours(2),
            emitted_at: t0 + chrono::Duration::hours(2),
            order_id: 3,
            trade_id: 3,
            account: account.to_string(),
            base: "BTC".into(),
            quote: "USD".into(),
            side: Side::Ask.to_string(),
            market_type: "spot".into(),
            price: dec!(50000),
            size: dec!(0.15),
            fee: dec!(7.5), // fee in USD for example
            fee_symbol: "USD".into(),
        };

        let latest_time = trade3.received_at;
        db.record_trades(vec![trade1, trade2, trade3])
            .await
            .unwrap();

        let trades = db.list_trades(account.clone()).await.unwrap();
        assert_eq!(trades.len(), 3);

        let exposures = db.load_exposures(account.clone()).await.unwrap();
        println!("{exposures:?}");

        // ---- Expected exposure math ----
        // base_exposure = +0.1 + 0.2 - 0.15 = +0.15 BTC
        // quote_exposure = -(0.1*45000) -(0.2*40000) + (0.15*50000)
        //                 = -4500 -8000 +7500
        //                 = -5000 USD

        let exposure = exposures.first().unwrap();
        assert_eq!(exposure.base, "BTC");
        assert_eq!(exposure.quote, "USD");
        assert_eq!(exposure.base_exposure, dec!(0.15));
        assert_eq!(exposure.quote_exposure, dec!(-5000));

        let date: DateTime<Utc> = exposure.latest_emitted_at.into();
        assert_eq!(date.timestamp_micros(), latest_time.timestamp_micros());
    }
}
