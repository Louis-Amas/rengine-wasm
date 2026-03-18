use crate::{
    execution_exchanges::ExecutionExchanges, reader_exchanges::Exchanges,
    strategies::StrategiesHandler, trades_exposures::TradesExposure,
    transformers::TransformersHandler,
};
use anyhow::{anyhow, Result};
use binance_perp::{BinancePerpPrivate, BinancePerpPublic};
use binance_spot::{BinanceSpotPrivate, BinanceSpotPublic};
use chrono::{DateTime, TimeZone, Utc};
use evm::{executor::EvmExecutor, EvmReaderHandler};
use futures::{future::join_all, FutureExt};
use hyperliquid::{private::HyperLiquidPrivate, public::HyperLiquidPublic};
use parking_lot::RwLock;
use rengine_config::{
    Config, EvmExecutionConfig, EvmReaderConfig, ExchangeConfig, ReaderExchangeConfig,
};
use rengine_db::{self, Db};
use rengine_interfaces::db::{AnalyticRepository, EvmLogsRepository, MultiCallRepository};
use rengine_metrics::latencies::{record_latency, LatencyId};
use rengine_non_wasm_types::{ChangesTx, TopBookRegistry};
use rengine_types::{
    db::{
        BalanceDb, ExposureDb, IndicatorDb, PositionDb, PublicTradeDb, Record, StorageDb,
        StrategyLogsDb, TopBookDb, TransformerLogsDb,
    },
    Account, Action, BalanceKey, BookKey, BulkCancelStatus, BulkPostStatus, EvmAccount,
    ExecutionRequest, ExecutionResult, Mapping, OpenOrder, OrderbookResults, PublicTrade,
    PublicTrades, State, StateUpdateKey, TopBookUpdate, Venue, VenueBookKey,
};
use rust_decimal::Decimal;
use std::{
    collections::{HashMap, HashSet},
    iter,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{
        mpsc::{channel, unbounded_channel, Receiver, UnboundedReceiver, UnboundedSender},
        oneshot, watch,
    },
    task::JoinHandle,
    time,
};
use tokio_stream::{wrappers::WatchStream, StreamExt, StreamMap};
use tracing::{error, info, warn};

pub type EvmReaders = Arc<HashMap<Venue, EvmReaderHandler>>;
type ActionResult = (
    Vec<Record>,
    HashSet<StateUpdateKey>,
    HashMap<VenueBookKey, Vec<PublicTrade>>,
);

pub struct Engine {
    state: Arc<RwLock<State>>,
    pub strategies_handler: StrategiesHandler,
    pub transformers_handler: TransformersHandler,
    changes_tx: ChangesTx,
    changes_rx: Receiver<Vec<Action>>,
    external_requests_rx: UnboundedReceiver<ExecutionRequest>,
    reader_exchanges: Exchanges,
    execution_exchanges: ExecutionExchanges,
    mappings: Mapping,
    spot_trades_exposures: TradesExposure,
    db: Db,
    handles: Vec<JoinHandle<Result<()>>>,
    save_public_trades: bool,
    top_book_streams: StreamMap<VenueBookKey, WatchStream<TopBookUpdate>>,
    register_rx: UnboundedReceiver<(VenueBookKey, watch::Receiver<TopBookUpdate>)>,
    top_book_registry: Arc<TopBookRegistry>,
    pub evm_readers: EvmReaders,
    pub evm_executors: HashMap<EvmAccount, Arc<EvmExecutor>>,
}

impl Engine {
    const BUFFER_SIZE: usize = 2048;
    #[cfg(test)]
    pub async fn new_test() -> Result<(Self, UnboundedSender<ExecutionRequest>)> {
        let config = Config::default();
        Self::new(config).await
    }
    async fn setup_evm_executors(
        &mut self,
        evm_executors_config: HashMap<EvmAccount, EvmExecutionConfig>,
    ) -> Result<()> {
        for (account, config) in evm_executors_config {
            info!("setup evm executor {account}");
            if let Some(handler) = self.evm_readers.get(&account.venue) {
                let executor = EvmExecutor::new_with_provider(
                    handler.reader.provider.clone(),
                    config.private_key,
                    handler.pending_tx_tx.clone(),
                    handler.reader.chain_id,
                    account.venue.clone(),
                )
                .await?;

                self.evm_executors.insert(account, Arc::new(executor));
            } else {
                error!("no evm reader for venue {}", account.venue);
            }
        }

        Ok(())
    }

    async fn setup_exchanges(
        &mut self,
        exchanges: HashMap<Account, ExchangeConfig>,
        readers: HashMap<Venue, ReaderExchangeConfig>,
    ) -> Result<()> {
        let registry = self.top_book_registry.clone();

        for (account, exchange) in exchanges {
            let exposures = self
                .spot_trades_exposures
                .load_account_exposure(account.clone())
                .await?;

            for (key, exposure) in exposures {
                info!(?exposure, "init_exposure exposure {key}");
                let _ = self
                    .state
                    .write()
                    .spot_exposures
                    .insert(key.clone(), exposure)
                    .unwrap_or_default();
            }

            info!("setup account {account}");
            match exchange {
                ExchangeConfig::Hyperliquid(config) => {
                    let exchange = HyperLiquidPrivate::new(
                        account.clone(),
                        config.clone(),
                        self.mappings.clone(),
                        self.changes_tx.clone(),
                    )
                    .await;

                    self.reader_exchanges.register_private_exchange(
                        account.clone(),
                        Arc::new(exchange.http),
                        config.private_exchange_polling_config,
                    );

                    self.execution_exchanges
                        .register(account, Arc::new(exchange.exchange));

                    self.handles.extend(exchange.handles);
                }
                ExchangeConfig::BinancePerp(config) => {
                    let (api_key, secret_key) = config.get_credentials()?;

                    let exchange = BinancePerpPrivate::new(
                        account.clone(),
                        self.changes_tx.clone(),
                        self.mappings.clone(),
                        api_key,
                        secret_key,
                    )
                    .await?;

                    self.execution_exchanges
                        .register(account, Arc::new(exchange.exchange));

                    self.handles.extend(exchange.handles);
                }
                ExchangeConfig::BinanceSpot(config) => {
                    let (api_key, secret_key) = config.get_credentials()?;

                    let exchange = BinanceSpotPrivate::new(
                        account.clone(),
                        self.changes_tx.clone(),
                        self.mappings.clone(),
                        api_key,
                        secret_key,
                    )
                    .await?;

                    self.execution_exchanges
                        .register(account, Arc::new(exchange.exchange));

                    self.handles.extend(exchange.handles);
                }
            }
        }

        for (venue, exchange) in readers {
            info!("setup venue {venue}");
            match exchange {
                ReaderExchangeConfig::Hyperliquid(config) => {
                    let exchange = HyperLiquidPublic::new(
                        venue.clone(),
                        config.clone(),
                        self.changes_tx.clone(),
                        self.mappings.clone(),
                        registry.clone(),
                    );

                    self.reader_exchanges
                        .register_public_exchange(
                            venue,
                            Arc::new(exchange.http),
                            config.public_exchange_polling_config,
                        )
                        .await;

                    self.handles.extend(exchange.handles);
                }
                ReaderExchangeConfig::BinanceSpot(config) => {
                    let exchange = BinanceSpotPublic::new(
                        venue.clone(),
                        self.changes_tx.clone(),
                        self.mappings.clone(),
                        registry.clone(),
                    )
                    .await;

                    self.reader_exchanges
                        .register_public_exchange(
                            venue,
                            Arc::new(exchange.http),
                            config.public_exchange_polling_config,
                        )
                        .await;

                    self.handles.extend(exchange.handles);
                }
                ReaderExchangeConfig::BinancePerp(config) => {
                    let exchange = BinancePerpPublic::new(
                        venue.clone(),
                        self.changes_tx.clone(),
                        self.mappings.clone(),
                        registry.clone(),
                    )
                    .await;

                    self.reader_exchanges
                        .register_public_exchange(
                            venue,
                            Arc::new(exchange.http),
                            config.public_exchange_polling_config,
                        )
                        .await;

                    self.handles.extend(exchange.handles);
                }
            }
        }

        Ok(())
    }

    async fn setup_evm_readers(
        repo: Arc<dyn MultiCallRepository>,
        evm_logs_repo: Arc<dyn EvmLogsRepository>,
        analytic_repo: Option<Arc<dyn AnalyticRepository>>,
        changes_tx: ChangesTx,
        state: Arc<RwLock<State>>,
        evm_readers_config: HashMap<Venue, EvmReaderConfig>,
    ) -> HashMap<Venue, EvmReaderHandler> {
        let futures = evm_readers_config.into_iter().map(|(venue, config)| {
            let state = state.clone();
            let changes_tx = changes_tx.clone();
            let repo_clone = repo.clone();
            let evm_logs_repo_clone = evm_logs_repo.clone();
            let analytic_repo = analytic_repo.clone();
            async move {
                match EvmReaderHandler::try_new(
                    venue.clone(),
                    config,
                    state,
                    repo_clone,
                    evm_logs_repo_clone,
                    analytic_repo,
                    changes_tx,
                )
                .await
                {
                    Ok(handler) => Ok((venue, handler)),
                    Err(err) => Err(err),
                }
            }
        });

        let results = join_all(futures).await;

        let mut handlers = HashMap::new();
        for res in results {
            match res {
                Ok((venue, handler)) => {
                    info!(%venue, "evm reader initialized");
                    handlers.insert(venue, handler);
                }
                Err(err) => {
                    error!(?err, "couldn't initialize evm reader");
                }
            }
        }

        handlers
    }

    pub async fn evm_handler(&self, venue: Venue) -> Option<&EvmReaderHandler> {
        self.evm_readers.get(&venue)
    }

    pub async fn new(config: Config) -> Result<(Self, UnboundedSender<ExecutionRequest>)> {
        let (tx, rx) = channel(Self::BUFFER_SIZE);
        let (external_requests_tx, external_requests_rx) = unbounded_channel::<ExecutionRequest>();

        let state: Arc<RwLock<State>> = Default::default();

        let save_public_trades = config
            .db
            .clickhouse
            .as_ref()
            .map(|c| c.save_public_trade)
            .unwrap_or(false);

        let db = Db::new(config.db).await?;

        let multicall_repo = db.duckdb.clone();
        let evm_logs_repo = db.duckdb.clone();

        let evm_readers = Self::setup_evm_readers(
            multicall_repo,
            evm_logs_repo,
            db.clickhouse
                .as_ref()
                .map(|clickhouse| clickhouse.clone() as Arc<dyn AnalyticRepository + 'static>),
            tx.clone(),
            state.clone(),
            config.evm_readers,
        )
        .await;

        let mappings = Mapping::new(config.mappings);
        let (top_book_registry, register_rx) = TopBookRegistry::new();

        let mut engine = Self {
            state: state.clone(),
            strategies_handler: StrategiesHandler::new(state.clone(), db.duckdb.clone()).await?,
            transformers_handler: TransformersHandler::new(state.clone(), db.duckdb.clone())
                .await?,
            changes_tx: tx.clone(),
            changes_rx: rx,
            external_requests_rx,
            reader_exchanges: Exchanges::new(tx.clone()),
            execution_exchanges: ExecutionExchanges::new(mappings.clone()),
            mappings,
            spot_trades_exposures: TradesExposure::new(db.duckdb.clone()),
            handles: Default::default(),
            evm_readers: Arc::new(evm_readers),
            evm_executors: Default::default(),
            save_public_trades,
            db,
            top_book_streams: StreamMap::new(),
            register_rx,
            top_book_registry,
        };

        engine.setup_evm_executors(config.evm_executors).await?;
        engine
            .setup_exchanges(config.exchanges, config.readers)
            .await?;
        info!("all exchanges initialized");

        // Wait a bit to read state from exchanges
        // tokio::time::sleep(Duration::from_secs(5)).await;

        Ok((engine, external_requests_tx))
    }

    pub fn state(&self) -> Arc<RwLock<State>> {
        self.state.clone()
    }

    pub async fn stop(&self) {
        self.db.stop().await;
        self.reader_exchanges.stop();

        for handle in &self.handles {
            handle.abort();
        }
    }

    fn handle_set_balance(
        &self,
        state: &mut State,
        time: DateTime<Utc>,
        key: BalanceKey,
        balance: Decimal,
    ) -> Record {
        state.balances.insert(key.clone(), balance);

        Record::Balance(BalanceDb {
            account: key.account.to_string(),
            balance,
            received_at: time,
            symbol: key.symbol,
        })
    }

    fn handle_set_top_book(
        &self,
        updated_keys: &mut HashSet<StateUpdateKey>,
        state: &mut State,
        time: DateTime<Utc>,
        mut venue_key: VenueBookKey,
        book: TopBookUpdate,
    ) -> Result<Record> {
        let instrument = self
            .mappings
            .map_instrument(&venue_key.venue, &venue_key.instrument)?;

        venue_key.instrument = instrument.key();
        let venue = venue_key.venue.clone();
        updated_keys.insert(StateUpdateKey::SetTopBook(venue_key.clone()));
        state.book.insert(venue_key, book.clone());

        Ok(Record::TopBook(TopBookDb {
            received_at: time,
            venue,
            base: instrument.base,
            quote: instrument.quote,
            market_type: instrument.market_type.to_string(),
            ask_price: book.top_ask.price,
            ask_size: book.top_ask.size,
            bid_price: book.top_bid.price,
            bid_size: book.top_ask.size,
        }))
    }

    fn handle_set_open_order(
        &self,
        state: &mut State,
        mut book_key: BookKey,
        id: String,
        open_order: OpenOrder,
    ) -> Result<()> {
        let instrument = self
            .mappings
            .map_instrument(&book_key.account.venue, &book_key.instrument)?;

        if instrument.market_type != book_key.account.market_type {
            return Ok(());
        }

        book_key.instrument = instrument.key();
        info!(?id, ?open_order, "handle set open order {book_key}");

        if !open_order.is_snapshot {
            if let Err(err) = self
                .execution_exchanges
                .remove_open_order(&book_key, &open_order)
            {
                warn!(?err)
            }
        }

        let open_orders = state.open_orders.entry(book_key).or_default();
        open_orders.insert(id, open_order.info);

        Ok(())
    }

    fn handle_remove_open_order(
        &self,
        state: &mut State,
        mut book_key: BookKey,
        id: String,
    ) -> Result<()> {
        let instrument = self
            .mappings
            .map_instrument(&book_key.account.venue, &book_key.instrument)?;

        if instrument.market_type != book_key.account.market_type {
            return Ok(());
        }

        info!(?id, "handle remove open order {book_key}");

        book_key.instrument = instrument.key();

        if let Some(open_orders) = state.open_orders.get_mut(&book_key) {
            open_orders.remove(&id);
        }

        Ok(())
    }

    fn handle_update_open_order(
        &self,
        state: &mut State,
        mut book_key: BookKey,
        id: String,
        size: Decimal,
    ) -> Result<()> {
        let instrument = self
            .mappings
            .map_instrument(&book_key.account.venue, &book_key.instrument)?;

        if instrument.market_type != book_key.account.market_type {
            return Ok(());
        }

        book_key.instrument = instrument.key();
        info!(?id, ?size, "handle update open order {book_key}");

        let open_orders = state.open_orders.get_mut(&book_key).ok_or_else(|| {
            anyhow!(
                "missing book key in perp_open_orders: {:?}",
                book_key.instrument
            )
        })?;

        if let Some(mut order) = open_orders.remove(&id) {
            order.size = size;
            if order.size > Decimal::ZERO {
                open_orders.insert(id, order);
            }
        }

        Ok(())
    }

    fn handle_set_perp_positon(
        &self,
        state: &mut State,
        mut book_key: BookKey,
        size: Decimal,
    ) -> Result<Record> {
        let instrument = self
            .mappings
            .map_instrument(&book_key.account.venue, &book_key.instrument)?;
        book_key.instrument = instrument.key();
        info!(?size, "handle set perp position {book_key}");

        // FIXME: improve position mapping
        let position_record = Record::Position(PositionDb {
            received_at: Utc::now(),
            venue: book_key.account.venue.clone(),
            account_id: book_key.account.account_id.clone(),
            symbol: instrument.key(),
            position_type: "margin".to_string(),
            side: if size.is_sign_positive() {
                "long".to_string()
            } else {
                "short".to_string()
            },
            size,
        });

        let entry = state.positions.entry(book_key).or_default();
        *entry = size;

        Ok(position_record)
    }

    fn handle_set_trade_flow(
        &self,
        updated_keys: &mut HashSet<StateUpdateKey>,
        aggregated_trades: &mut HashMap<VenueBookKey, Vec<PublicTrade>>,
        mut key: VenueBookKey,
        trades: PublicTrades,
    ) -> Result<Vec<Record>> {
        let instrument = self.mappings.map_instrument(&key.venue, &key.instrument)?;

        key.instrument = instrument.key();
        updated_keys.insert(StateUpdateKey::SetTradeFlow(key.clone()));

        let records = trades
            .data
            .iter()
            .map(|trade| {
                Record::PublicTrade(PublicTradeDb {
                    received_at: Utc::now(),
                    venue: key.venue.clone(),
                    instrument: key.instrument.clone(),
                    price: trade.price,
                    size: trade.size,
                    side: trade.side.to_string(),
                    time: Utc.timestamp_millis_opt(trade.time as i64).unwrap(),
                    trade_id: trade.trade_id.clone(),
                })
            })
            .collect();

        // Aggregate trades instead of overwriting
        aggregated_trades
            .entry(key)
            .or_default()
            .extend(trades.data);

        Ok(records)
    }

    fn handle_actions(&mut self, actions: Vec<Action>) -> Result<ActionResult> {
        let mut state = self.state.write();
        let mut records = Vec::new();
        let mut aggregated_trades: HashMap<VenueBookKey, Vec<PublicTrade>> = HashMap::new();

        let mut updated_keys = <_>::default();

        let now = Utc::now();

        for action in actions {
            match action {
                Action::SetBalance(key, balance) => {
                    let record = self.handle_set_balance(&mut state, now, key, balance);
                    records.push(record);
                }
                Action::SetIndicator(key, indicator) => {
                    state.indicators.insert(key.clone(), indicator);

                    records.push(Record::Indicator(IndicatorDb {
                        set_at: Utc::now(),
                        key,
                        value: indicator,
                    }));
                }
                Action::SetTopBook(book_key, book) => {
                    match self.handle_set_top_book(
                        &mut updated_keys,
                        &mut state,
                        now,
                        book_key,
                        book,
                    ) {
                        Ok(record) => records.push(record),
                        Err(err) => error!("couldn't create top book record {err}"),
                    };
                }
                Action::SetOpenOrder(book_key, id, open_order) => {
                    if let Err(err) =
                        self.handle_set_open_order(&mut state, book_key, id, open_order)
                    {
                        warn!("couldn't send open order {err:?}");
                    };
                }
                Action::RemoveOpenOrder(book_key, id) => {
                    if let Err(err) = self.handle_remove_open_order(&mut state, book_key, id) {
                        warn!("couldn't remove open order {err:?}");
                    };
                }
                Action::UpdateOpenOrder(book_key, id, size) => {
                    if let Err(err) = self.handle_update_open_order(&mut state, book_key, id, size)
                    {
                        warn!("couldn't update open order {err:?}");
                    };
                }
                Action::SetPerpPosition(book_key, size) => {
                    match self.handle_set_perp_positon(&mut state, book_key, size) {
                        Ok(record) => records.push(record),
                        Err(err) => warn!("couldn't set perp position {err:?}"),
                    };
                }
                Action::SetTradeFlow(key, trades) => {
                    match self.handle_set_trade_flow(
                        &mut updated_keys,
                        &mut aggregated_trades,
                        key,
                        trades,
                    ) {
                        Ok(new_records) => {
                            if self.save_public_trades {
                                records.extend(new_records);
                            }
                        }
                        Err(err) => warn!("couldn't set trade flow {err:?}"),
                    }
                }
                Action::RecordTrades(trades) => {
                    let trades_values = trades.iter().map(|(_, trade)| trade);

                    let (exposures, trades) =
                        self.spot_trades_exposures.apply_trades(trades_values);

                    for (key, exposure) in exposures {
                        records.push(Record::Exposure(ExposureDb {
                            set_at: Utc::now(),
                            account: key.account.to_string(),
                            symbol: key.symbol.clone(),
                            balance: exposure,
                        }));
                        info!(?exposure, "new exposure {key}");
                        let old_exposure = state
                            .spot_exposures
                            .insert(key.clone(), exposure)
                            .unwrap_or_default();
                        info!(?old_exposure, "old exposure {key}");
                    }

                    records.extend(trades.into_iter().map(Record::Trade));
                }
                Action::HandleExecutionResult(results) => {
                    let ExecutionResult::Orderbook(results) = results;
                    match results.data {
                        OrderbookResults::BulkPost(post_results) => {
                            for post in post_results {
                                match &post.status {
                                    BulkPostStatus::Error(err) => {
                                        error!(?post, ?err, "couldn't post");
                                    }
                                    _ => info!("success post {post:?}"),
                                }
                            }
                        }
                        OrderbookResults::BulkCancel(cancel_results) => {
                            for cancel in cancel_results {
                                match &cancel.status {
                                    BulkCancelStatus::Error(err) => {
                                        error!(?cancel, ?err, "couldn't cancel");
                                    }
                                    _ => info!(?cancel, "success cancel"),
                                }
                            }
                        }
                    }
                }
                Action::SetMarketSpec(key, spec) => {
                    state.market_specs.insert(key, spec);
                }
                Action::SetStorage(key, value) => {
                    state.storage.insert(key.clone(), value.clone());

                    records.push(Record::Storage(StorageDb {
                        set_at: Utc::now(),
                        key,
                        value,
                    }));
                }
            }
        }

        Ok((records, updated_keys, aggregated_trades))
    }

    pub async fn handle_requests<I>(&self, requests: I) -> Vec<Record>
    where
        I: Iterator<Item = ExecutionRequest>,
    {
        let futures_and_requests: Vec<_> = requests
            .map(|request| {
                let fut = match &request {
                    ExecutionRequest::EvmTx((account, tx)) => {
                        if let Some(executor) = self.evm_executors.get(account) {
                            let executor = executor.clone();
                            let tx = tx.clone();
                            async move {
                                match executor.execute_evm_tx(tx.0.clone()).await {
                                    Ok(record) => Ok(record),
                                    Err(e) => {
                                        error!(?e, "failed to execute evm tx");
                                        Ok(None)
                                    }
                                }
                            }
                            .boxed()
                        } else {
                            async move { Err(anyhow!("no evm executor for account")) }.boxed()
                        }
                    }
                    _ => {
                        let fut = self
                            .execution_exchanges
                            .handle_execution_request(request.clone());
                        async move { fut.await.map_err(|e| anyhow!(e)).map(|_| None) }.boxed()
                    }
                };
                (fut, request)
            })
            .collect();

        let (futures, requests): (Vec<_>, Vec<_>) = futures_and_requests.into_iter().unzip();

        let results = join_all(futures).await;

        let mut records = Vec::new();

        for (result, request) in results.into_iter().zip(requests) {
            info!(?request, "successfully handled request");
            match result {
                Ok(Some(record)) => records.push(record),
                Ok(None) => {}
                Err(err) => {
                    error!(?request, ?err, "error handling request");
                }
            }
        }
        records
    }

    pub async fn handle_changes(
        &mut self,
        size: usize,
        actions: &mut Vec<Vec<Action>>,
    ) -> Result<()> {
        let instant = Instant::now();
        let actions: Vec<Action> = actions.drain(..size).flatten().collect();

        let (mut records, mut updated_keys, aggregated_trades) = self.handle_actions(actions)?;
        record_latency(LatencyId::HandleChanges, instant);

        let mut all_requests = vec![];

        // Transformers execution
        let instant = Instant::now();
        let mut transformer_actions = Vec::new();

        // We need to clone updated_keys because we need them for strategies later,
        // and we might add more keys from transformers.
        for result in self
            .transformers_handler
            .execute_transformers(&updated_keys, instant, aggregated_trades)
            .await
        {
            if !result.execution_result.logs.is_empty() {
                let transformer_db = TransformerLogsDb::try_from(&result)?;
                records.push(Record::TransformerLog(transformer_db));
            }

            for request in result.execution_result.requests {
                match request {
                    ExecutionRequest::SetIndicator(key, value) => {
                        transformer_actions.push(Action::SetIndicator(key, value));
                    }
                    other => all_requests.push(other),
                }
            }
        }

        if !transformer_actions.is_empty() {
            let (new_records, new_keys, _) = self.handle_actions(transformer_actions)?;
            records.extend(new_records);
            updated_keys.extend(new_keys);
        }
        record_latency(LatencyId::TransformersExecution, instant);

        let instant = Instant::now();
        for result in self
            .strategies_handler
            .execute_strategies(updated_keys, instant)
            .await
        {
            if !result.execution_result.requests.is_empty()
                || !result.execution_result.logs.is_empty()
            {
                let strategy_db = StrategyLogsDb::try_from(&result)?;
                records.push(Record::StrategyLog(strategy_db));
            }

            all_requests.extend(result.execution_result.requests.into_iter());
        }
        record_latency(LatencyId::StrategiesExecution, instant);

        let instant = Instant::now();
        let request_records = self.handle_requests(all_requests.into_iter()).await;
        records.extend(request_records);
        record_latency(LatencyId::HandleRequests, instant);

        if self.db.clickhouse.is_some() {
            let clickhouse_arc = self.db.clickhouse.clone();
            tokio::spawn(async move {
                if let Some(clickhouse) = clickhouse_arc {
                    if let Err(err) = clickhouse.batch_insert(records).await {
                        error!(?err, "couldn't batch insert record");
                    }
                }
            });
        }

        Ok(())
    }

    pub async fn crank(&mut self, mut shutdown: oneshot::Receiver<()>) -> Result<()> {
        let mut actions = Vec::with_capacity(Self::BUFFER_SIZE);

        loop {
            let outer_instant = Instant::now();
            tokio::select! {
                _ = &mut shutdown => {
                    break;
                }
                Some((key, rx)) = self.register_rx.recv() => {
                    self.top_book_streams.insert(key, WatchStream::new(rx));
                }
                Some((key, update)) = self.top_book_streams.next() => {
                    let action = Action::SetTopBook(key, update);
                    actions.push(vec![action]);

                    loop {
                        if actions.len() >= Self::BUFFER_SIZE {
                            break;
                        }

                        match self.top_book_streams.next().now_or_never() {
                            Some(Some((key, update))) => {
                                let action = Action::SetTopBook(key, update);
                                actions.push(vec![action]);
                            }
                            _ => break,
                        }
                    }

                    if let Err(err) = self.handle_changes(actions.len(), &mut actions).await {
                        error!(?err, "couldn't handle changes from top book stream");
                    }
                    actions.clear();
                }
                size = self.changes_rx.recv_many(&mut actions, Self::BUFFER_SIZE) => {
                    if size == 0 {
                        break;
                    }
                    let instant = Instant::now();

                    if let Err(err) = self.handle_changes(size, &mut actions).await {
                        error!(?err, "couldn't handle changes");
                    }


                    actions.clear();
                    record_latency(LatencyId::CrankLoop, instant);
                }
                Some(request) = self.external_requests_rx.recv() => {
                    let instant = Instant::now();
                    let records = self.handle_requests(iter::once(request)).await;
                    if let Some(clickhouse) = &self.db.clickhouse {
                        if let Err(err) = clickhouse.batch_insert(records).await {
                            error!(?err, "couldn't batch insert record");
                        }
                    }
                    record_latency(LatencyId::HandleRequests, instant);
                }
                _ = time::sleep(Duration::from_millis(10)) => {
                    // We want to trigger the timers
                    if let Err(err) = self.handle_changes(0, &mut actions).await {
                        error!(?err, "couldn't handle changes");
                    }
                }
            }
            record_latency(LatencyId::OuterCrankLoop, outer_instant);
        }

        self.stop().await;
        Ok(())
    }
}
