use anyhow::{anyhow, bail, Result};
use rengine_interfaces::ExchangeExecution;
use rengine_metrics::counters::increment_counter;
use rengine_types::{
    Account, BookKey, ExecutionRequest, ExecutionType, Instrument, Mapping, OpenOrder,
    OrderActions, OrderInfo, OrderReference,
};
use std::{
    collections::HashMap,
    fmt,
    hash::Hash,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use thiserror::Error;
use tracing::debug;

/// A small error enum to wrap “unknown venue” and any exchange‐side failures.
#[derive(Debug, Error)]
pub(crate) enum ExecutionError {
    #[error("no exchange registered for venue {0:?}")]
    UnknownAccount(Account),
    #[error(transparent)]
    Exchange(#[from] anyhow::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OrderCacheKey {
    instrument: Instrument,
    info: OrderInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CancelCacheKey {
    instrument: Instrument,
    order_id: OrderReference,
}

type VenueLock<K> = Arc<Mutex<HashMap<K, Instant>>>;

pub(crate) struct ExecutionExchanges {
    mappings: Mapping,
    inner: HashMap<Account, Arc<dyn ExchangeExecution + Send + Sync>>,
    post_cache: Mutex<HashMap<Account, VenueLock<OrderCacheKey>>>,
    cancel_cache: Mutex<HashMap<Account, VenueLock<CancelCacheKey>>>,
}

impl ExecutionExchanges {
    pub(crate) fn new(mapping: Mapping) -> Self {
        Self {
            mappings: mapping,
            inner: <_>::default(),
            post_cache: <_>::default(),
            cancel_cache: <_>::default(),
        }
    }

    /// Register one exchange implementation under its venue key
    pub(crate) fn register<E>(&mut self, account: Account, exec: Arc<E>)
    where
        E: ExchangeExecution + Send + Sync + 'static,
    {
        self.inner.insert(account, exec);
    }

    fn get_or_insert_cache<K: Eq + Hash + Clone + Send>(
        cache_map: &Mutex<HashMap<Account, VenueLock<K>>>,
        account: &Account,
    ) -> Result<Arc<Mutex<HashMap<K, Instant>>>> {
        let result = cache_map
            .lock()
            .map_err(|err| anyhow!("couldn't lock {err:?}"))?
            .entry(account.clone())
            .or_default()
            .clone();

        Ok(result)
    }

    /// Generic TTL-based filter for any key/item pair
    fn filter_new<K, T, F>(
        cache: Arc<Mutex<HashMap<K, Instant>>>,
        max_dur: Duration,
        items: Vec<T>,
        make_key: F,
    ) -> Result<Vec<T>>
    where
        K: Eq + Hash + Clone + Send + fmt::Debug,
        T: Clone + Send + fmt::Debug,
        F: Fn(&T) -> K + Send + Sync,
    {
        let now = Instant::now();
        let mut to_send = Vec::new();
        let mut locked = cache
            .lock()
            .map_err(|err| anyhow!("couldn't lock venue lock {err:?}"))?;

        // clean timeout expired
        locked.retain(|_, &mut ts| now.duration_since(ts) < max_dur);

        // TODO: this prevent to have multiple in flights per venue, it needs to be improved to
        // be by instrument and side
        if !locked.is_empty() {
            return Ok(Default::default());
        }

        for item in items {
            let key = make_key(&item);
            locked.insert(key, now);
            to_send.push(item);
        }

        Ok(to_send)
    }

    pub(crate) fn remove_open_order(
        &self,
        book_key: &BookKey,
        open_order: &OpenOrder,
    ) -> Result<()> {
        let cache = Self::get_or_insert_cache(&self.post_cache, &book_key.account)?;
        let mut map = cache
            .lock()
            .map_err(|err| anyhow!("couldn't lock venue lock {err:?}"))?;

        let key = OrderCacheKey {
            instrument: book_key.instrument.clone(),
            info: open_order.info.clone(),
        };
        if map.remove(&key).is_none() {
            bail!("couldn't remove pending order from cache {key:?} {map:?}");
        }

        debug!(?key, ?open_order, "remove open order from cache");

        Ok(())
    }

    async fn handle_order_action(&self, mut action: OrderActions) -> Result<(), ExecutionError> {
        match &mut action {
            OrderActions::BulkPost((account, orders, execution_type)) => {
                let label = account.venue.as_str();
                for (instrument, _order) in &mut *orders {
                    let mapping = self
                        .mappings
                        .reverse_map_instrument(&account.venue, instrument)?;

                    *instrument = mapping;
                }

                let exec = self
                    .inner
                    .get(account)
                    .ok_or_else(|| ExecutionError::UnknownAccount(account.clone()))?;

                if *execution_type == ExecutionType::Unmanaged {
                    match exec.post_orders(orders.clone()).await {
                        Ok(_) => {
                            increment_counter(format!("{}|execution", label));
                            return Ok(());
                        }
                        Err(e) => {
                            increment_counter(format!("{}|execution", label));
                            return Err(e.into());
                        }
                    }
                }

                let max_dur = exec.max_response_duration();
                let cache = Self::get_or_insert_cache(&self.post_cache, account)?;

                let to_send = Self::filter_new(cache, max_dur, orders.clone(), |(inst, info)| {
                    OrderCacheKey {
                        instrument: inst.clone(),
                        info: info.clone(),
                    }
                })?;

                if to_send.is_empty() {
                    return Ok(());
                }

                match exec.post_orders(to_send).await {
                    Ok(_) => {
                        increment_counter(format!("{}|execution", label));
                        Ok(())
                    }
                    Err(e) => {
                        increment_counter(format!("{}|execution", label));
                        Err(e.into())
                    }
                }
            }

            OrderActions::BulkCancel((account, cancels, execution_type)) => {
                let label = account.venue.as_str();
                for (instrument, _order) in &mut *cancels {
                    let mapping = self
                        .mappings
                        .reverse_map_instrument(&account.venue, instrument)?;

                    *instrument = mapping;
                }

                let exec = self
                    .inner
                    .get(account)
                    .ok_or_else(|| ExecutionError::UnknownAccount(account.clone()))?;

                if *execution_type == ExecutionType::Unmanaged {
                    match exec.cancel_orders(cancels.clone()).await {
                        Ok(_) => {
                            increment_counter(format!("{}|execution", label));
                            return Ok(());
                        }
                        Err(e) => {
                            increment_counter(format!("{}|execution", label));
                            return Err(e.into());
                        }
                    }
                }

                let max_dur = exec.max_response_duration();
                let cache = Self::get_or_insert_cache(&self.cancel_cache, account)?;

                let to_send =
                    Self::filter_new(cache, max_dur, cancels.clone(), |(inst, order_id)| {
                        CancelCacheKey {
                            instrument: inst.clone(),
                            order_id: order_id.clone(),
                        }
                    })?;

                if to_send.is_empty() {
                    return Ok(());
                }

                match exec.cancel_orders(to_send).await {
                    Ok(_) => {
                        increment_counter(format!("{}|execution", label));
                        Ok(())
                    }
                    Err(e) => {
                        increment_counter(format!("{}|execution", label));
                        Err(e.into())
                    }
                }
            }
        }
    }

    /// Handle the top‑level enum of different execution requests
    pub(crate) async fn handle_execution_request(
        &self,
        req: ExecutionRequest,
    ) -> Result<(), ExecutionError> {
        if let ExecutionRequest::Orderbook(action) = req {
            return self.handle_order_action(action).await;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rengine_interfaces::test_utils::MockExchange;
    use rengine_types::{
        Instrument, Mapping, MarketType, OrderInfo, OrderReference, Side, TimeInForce, Venue,
    };
    use rust_decimal_macros::dec;
    use std::{sync::Arc, time::Duration};
    use tokio::time;

    #[tokio::test]
    async fn test_bulk_post_filters_and_ttl() {
        // Setup
        let mut execs = ExecutionExchanges::new(<_>::default());
        let venue: Venue = "test".into();
        let instrument: Instrument = "eth/usdc-spot".into();
        let account = Account {
            venue,
            account_id: "test".into(),
            market_type: MarketType::Spot,
        };

        let info = OrderInfo {
            size: dec!(10),
            price: dec!(100.0),
            side: Side::Ask,
            tif: TimeInForce::PostOnly,
            client_order_id: None,
            order_type: Default::default(),
        };

        let max_dur = Duration::from_millis(50);

        let instrument_mappings = Mapping::default();
        let mock = Arc::new(MockExchange::new(instrument_mappings, max_dur));
        execs.register(account.clone(), mock.clone());

        // First post: should go through.
        let req = ExecutionRequest::Orderbook(OrderActions::BulkPost((
            account.clone(),
            vec![(instrument.clone(), info.clone())],
            ExecutionType::Managed,
        )));
        execs.handle_execution_request(req).await.unwrap();
        {
            let posted = mock.posted_orders.lock().unwrap();
            assert_eq!(posted.len(), 1);
            assert_eq!(posted[0], (instrument.clone(), info.clone()));
        }

        // Immediate second post: filtered by TTL, so no new call.
        let req2 = ExecutionRequest::Orderbook(OrderActions::BulkPost((
            account.clone(),
            vec![(instrument.clone(), info.clone())],
            ExecutionType::Managed,
        )));
        execs.handle_execution_request(req2).await.unwrap();
        {
            let posted = mock.posted_orders.lock().unwrap();
            assert_eq!(posted.len(), 1, "Second post should be filtered");
        }

        // Wait for TTL to expire...
        time::sleep(max_dur).await;

        // Third post after TTL: should go through again.
        let req3 = ExecutionRequest::Orderbook(OrderActions::BulkPost((
            account.clone(),
            vec![(instrument.clone(), info.clone())],
            ExecutionType::Managed,
        )));
        execs.handle_execution_request(req3).await.unwrap();
        {
            let posted = mock.posted_orders.lock().unwrap();
            assert_eq!(posted.len(), 2);
        }

        let open_order = OpenOrder {
            info: info.clone(),
            original_size: info.size,
            is_snapshot: false,
        };

        execs
            .remove_open_order(
                &BookKey {
                    account: account.clone(),
                    instrument,
                },
                &open_order,
            )
            .unwrap();

        let lock = execs.post_cache.lock().unwrap();

        let map_lock = lock.get(&account).unwrap().lock().unwrap();

        assert_eq!(map_lock.len(), 0);
    }

    #[tokio::test]
    async fn test_bulk_cancel_filters_and_ttl() {
        let mut execs = ExecutionExchanges::new(<_>::default());
        let venue: Venue = "test".into();
        let instrument: Instrument = "eth/usdc-spot".into();
        let order_id = 42;
        let max_dur = Duration::from_millis(50);
        let instrument_mappings = Mapping::default();
        let mock = Arc::new(MockExchange::new(instrument_mappings, max_dur));
        let account = Account {
            venue,
            account_id: "test".into(),
            market_type: MarketType::Spot,
        };

        execs.register(account.clone(), mock.clone());

        // First cancel: should go through.
        execs
            .handle_execution_request(ExecutionRequest::Orderbook(OrderActions::BulkCancel((
                account.clone(),
                vec![(
                    instrument.clone(),
                    OrderReference::ExternalOrderId(order_id.to_string().into()),
                )],
                ExecutionType::Managed,
            ))))
            .await
            .unwrap();
        assert_eq!(mock.cancelled_orders.lock().unwrap().len(), 1);

        // Immediate second cancel: filtered by TTL.
        execs
            .handle_execution_request(ExecutionRequest::Orderbook(OrderActions::BulkCancel((
                account.clone(),
                vec![(
                    instrument.clone(),
                    OrderReference::ExternalOrderId(order_id.to_string().into()),
                )],
                ExecutionType::Managed,
            ))))
            .await
            .unwrap();
        assert_eq!(mock.cancelled_orders.lock().unwrap().len(), 1);

        time::sleep(max_dur).await;
        // After TTL: should go through again.
        execs
            .handle_execution_request(ExecutionRequest::Orderbook(OrderActions::BulkCancel((
                account.clone(),
                vec![(
                    instrument.clone(),
                    OrderReference::ExternalOrderId(order_id.to_string().into()),
                )],
                ExecutionType::Managed,
            ))))
            .await
            .unwrap();
        assert_eq!(mock.cancelled_orders.lock().unwrap().len(), 2);
    }
}
