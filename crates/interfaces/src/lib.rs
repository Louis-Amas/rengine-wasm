use anyhow::Result;
use async_trait::async_trait;
use futures::{future::join_all, FutureExt};
use rengine_non_wasm_types::{send_changes, ChangesTx};
use rengine_types::{
    Action, BalanceKey, BookKey, Decimal, Instrument, OrderInfo, OrderReference, Trade,
};
use std::time::Duration;
use tracing::error;
pub mod db;

pub fn handle_balances(changes_tx: &ChangesTx, balances: Vec<(BalanceKey, Decimal)>) {
    let actions = balances
        .into_iter()
        .map(|(key, value)| Action::SetBalance(key, value))
        .collect();
    send_changes(changes_tx, actions);
}

pub fn handle_trades(changes_tx: &ChangesTx, trades: Vec<(BookKey, Trade)>) {
    let action = Action::RecordTrades(trades);

    send_changes(changes_tx, vec![action]);
}

#[async_trait]
pub trait PublicExchangeReader {
    async fn fetch_book(&self) -> Result<()>;
    async fn fetch_funding(&self) -> Result<()>;
    async fn fetch_market_specs(&self) -> Result<()>;
}

#[async_trait]
pub trait ExchangePrivateReader {
    async fn fetch_open_orders(&self) -> Result<()>;
    async fn fetch_balances(&self) -> Result<Vec<(BalanceKey, Decimal)>>;
    async fn fetch_trades(&self) -> Result<Vec<(BookKey, Trade)>>;
    async fn fetch_positions(&self) -> Result<()>;

    async fn sync_state(&self, changes_tx: &ChangesTx) -> Result<()> {
        let futures = vec![
            self.fetch_open_orders().boxed(),
            self.fetch_positions().boxed(),
            self.fetch_balances()
                .map(|res| {
                    handle_balances(changes_tx, res?);
                    Ok(())
                })
                .boxed(),
            self.fetch_trades()
                .map(|res| {
                    handle_trades(changes_tx, res?);
                    Ok(())
                })
                .boxed(),
        ];

        let results = join_all(futures).await;

        for result in results {
            if let Err(err) = result {
                error!("failed: {err:?}");
            }
        }

        Ok(())
    }
}

#[async_trait]
pub trait ExchangeExecution {
    async fn post_orders(&self, orders: Vec<(Instrument, OrderInfo)>) -> Result<()>;
    async fn cancel_orders(&self, cancels: Vec<(Instrument, OrderReference)>) -> Result<()>;
    fn max_response_duration(&self) -> Duration;
}

#[cfg(feature = "test_utils")]
pub mod test_utils {
    use super::{ExchangeExecution, ExchangePrivateReader};
    use crate::PublicExchangeReader;
    use anyhow::Result;
    use async_trait::async_trait;
    use rengine_types::{
        BalanceKey, BookKey, Decimal, Instrument, Mapping, OrderInfo, OrderReference, Trade,
    };
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Mutex,
        },
        time::Duration,
    };

    /// A mock exchange that records calls and captures orders/cancels.
    pub struct MockExchange {
        /// Flags so you can assert these fns ran.
        pub fetch_open_orders_called: AtomicBool,
        pub fetch_book_called: AtomicBool,
        pub fetch_positions_called: AtomicBool,

        /// Captured inputs to `post_orders` / `cancel_orders`.
        pub posted_orders: Mutex<Vec<(Instrument, OrderInfo)>>,
        pub cancelled_orders: Mutex<Vec<(Instrument, OrderReference)>>,

        /// What `instrument_mapping()` returns
        pub mapping: Mapping,
        /// What `max_response_duration()` returns
        pub max_duration: Duration,
    }

    impl MockExchange {
        /// Create a new mock with the given mapping & max‐duration.
        pub const fn new(mapping: Mapping, max_duration: Duration) -> Self {
            Self {
                fetch_open_orders_called: AtomicBool::new(false),
                fetch_book_called: AtomicBool::new(false),
                fetch_positions_called: AtomicBool::new(false),
                posted_orders: Mutex::new(Vec::new()),
                cancelled_orders: Mutex::new(Vec::new()),
                mapping,
                max_duration,
            }
        }
    }

    #[async_trait]
    impl PublicExchangeReader for MockExchange {
        async fn fetch_book(&self) -> Result<()> {
            self.fetch_book_called.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn fetch_funding(&self) -> Result<()> {
            Ok(())
        }

        async fn fetch_market_specs(&self) -> Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ExchangePrivateReader for MockExchange {
        async fn fetch_open_orders(&self) -> Result<()> {
            self.fetch_open_orders_called.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn fetch_balances(&self) -> Result<Vec<(BalanceKey, Decimal)>> {
            todo!();
        }

        async fn fetch_trades(&self) -> Result<Vec<(BookKey, Trade)>> {
            todo!();
        }
        async fn fetch_positions(&self) -> Result<()> {
            self.fetch_positions_called.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait]
    impl ExchangeExecution for MockExchange {
        async fn post_orders(&self, orders: Vec<(Instrument, OrderInfo)>) -> Result<()> {
            let mut guard = self.posted_orders.lock().unwrap();
            guard.extend(orders);
            Ok(())
        }

        async fn cancel_orders(&self, cancels: Vec<(Instrument, OrderReference)>) -> Result<()> {
            let mut guard = self.cancelled_orders.lock().unwrap();
            guard.extend(cancels);
            Ok(())
        }

        fn max_response_duration(&self) -> Duration {
            self.max_duration
        }
    }
}
