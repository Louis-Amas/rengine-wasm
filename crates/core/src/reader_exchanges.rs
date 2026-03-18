use anyhow::Result;
use rengine_interfaces::{
    handle_balances, handle_trades, ExchangePrivateReader, PublicExchangeReader,
};
use rengine_non_wasm_types::ChangesTx;
use rengine_types::{Account, PrivateExchangePollingConfig, PublicExchangePollingConfig, Venue};
use std::{collections::HashMap, future::Future, sync::Arc, time::Duration};
use tokio::{task::JoinHandle, time};
use tracing::{error, info};

pub(crate) struct Exchanges {
    public_readers: HashMap<Venue, Arc<dyn PublicExchangeReader + Send + Sync + 'static>>,
    exchange_reader: HashMap<Account, Arc<dyn ExchangePrivateReader + Send + Sync + 'static>>,
    changes_tx: ChangesTx,
    handles: Vec<JoinHandle<()>>,
}

impl Exchanges {
    pub(crate) fn new(changes_tx: ChangesTx) -> Self {
        Self {
            public_readers: Default::default(),
            exchange_reader: Default::default(),
            changes_tx,
            handles: Default::default(),
        }
    }

    pub(crate) fn stop(&self) {
        for handle in &self.handles {
            handle.abort();
        }
    }
    pub(crate) async fn register_public_exchange<E>(
        &mut self,
        venue: Venue,
        reader: Arc<E>,
        config: PublicExchangePollingConfig,
    ) where
        E: PublicExchangeReader + Send + Sync + 'static,
    {
        self.public_readers.insert(venue.clone(), reader.clone());
        let label_key = venue.to_string();

        if let Err(err) = reader.fetch_market_specs().await {
            error!("failed to fetch market specs on init: {:?}", err);
        }

        if let Some(interval) = config.fetch_book_interval {
            self.handles.push(Self::spawn_polling_task(
                label_key.clone(),
                "fetch_book",
                interval,
                {
                    let reader = reader.clone();
                    move || {
                        let reader = reader.clone();
                        async move { reader.fetch_book().await }
                    }
                },
            ));
        }

        if let Some(interval) = config.fetch_funding_interval {
            self.handles.push(Self::spawn_polling_task(
                label_key,
                "fetch_funding",
                interval,
                {
                    let reader = reader;
                    move || {
                        let reader = reader.clone();
                        async move { reader.fetch_funding().await }
                    }
                },
            ));
        }
    }

    pub(crate) fn register_private_exchange<E>(
        &mut self,
        account: Account,
        reader: Arc<E>,
        config: PrivateExchangePollingConfig,
    ) where
        E: ExchangePrivateReader + Send + Sync + 'static,
    {
        let label_key = account.to_string();
        self.exchange_reader.insert(account, reader.clone());
        let tx = self.changes_tx.clone();

        if let Some(interval) = config.fetch_open_orders_interval {
            self.handles.push(Self::spawn_polling_task(
                label_key.clone(),
                "fetch_open_orders",
                interval,
                {
                    let reader = reader.clone();
                    move || {
                        let reader = reader.clone();
                        async move { reader.fetch_open_orders().await }
                    }
                },
            ));
        }

        if let Some(interval) = config.fetch_positions_interval {
            self.handles.push(Self::spawn_polling_task(
                label_key.clone(),
                "fetch_positions",
                interval,
                {
                    let reader = reader.clone();
                    move || {
                        let reader = reader.clone();
                        async move { reader.fetch_positions().await }
                    }
                },
            ));
        }

        if let Some(interval) = config.fetch_balances_interval {
            self.handles.push(Self::spawn_polling_task_with_handler(
                label_key.clone(),
                "fetch_balances",
                interval,
                {
                    let reader = reader.clone();
                    move || {
                        let reader = reader.clone();
                        async move { reader.fetch_balances().await }
                    }
                },
                handle_balances,
                tx.clone(),
            ));
        }

        if let Some(interval) = config.fetch_trades_interval {
            self.handles.push(Self::spawn_polling_task_with_handler(
                label_key,
                "fetch_trades",
                interval,
                {
                    let reader = reader;
                    move || {
                        let reader = reader.clone();
                        async move { reader.fetch_trades().await }
                    }
                },
                handle_trades,
                tx,
            ));
        }
    }

    fn spawn_polling_task<F, Fut>(
        label_key: String,
        label: &'static str,
        interval: Duration,
        mut f: F,
    ) -> JoinHandle<()>
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        info!("[{label_key}] {label} initialized with interval {interval:?}");
        tokio::spawn(async move {
            let mut ticker = time::interval(interval);
            loop {
                ticker.tick().await;
                if let Err(err) = f().await {
                    error!("[{label_key}] {label} error: {err:?}");
                }
            }
        })
    }

    fn spawn_polling_task_with_handler<T, F, Fut, H>(
        label_key: String,
        label: &'static str,
        interval: Duration,
        mut f: F,
        handler: H,
        changes_tx: ChangesTx,
    ) -> JoinHandle<()>
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        H: Fn(&ChangesTx, T) + Send + Sync + 'static,
        T: Send + 'static,
    {
        info!("[{label_key}] {label} initialized with interval {interval:?}");
        tokio::spawn(async move {
            let mut ticker = time::interval(interval);
            loop {
                ticker.tick().await;
                match f().await {
                    Ok(value) => handler(&changes_tx, value),
                    Err(err) => error!("[{label_key}] {label} error: {err:?}"),
                }
            }
        })
    }
}
