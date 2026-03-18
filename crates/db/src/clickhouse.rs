use crate::migrate::Migration;
use anyhow::Result;
use chrono::Utc;
use clickhouse::inserter::Inserter;
use rengine_config::ClickHouseConfig;
use rengine_interfaces::db::AnalyticRepository;
use rengine_metrics::{
    counters::get_all_counters,
    latencies::{get_all_nonempty_stats, reset_all_latencies},
};
use rengine_types::db::{
    BalanceDb, CounterRow, EvmTxDb, ExposureDb, IndicatorDb, LatencySnapshotRow, OpenOrderDb,
    PositionDb, PublicTradeDb, Record, StorageDb, StrategyLogsDb, TopBookDb, TradeDb,
    TransformerLogsDb,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::interval,
};
use tracing::error;

#[allow(clippy::large_enum_variant)]
enum DbMessage {
    Insert(Record),
    Batch(Vec<Record>),
    Flush(oneshot::Sender<Result<()>>),
}

struct BackgroundInserters {
    top_book: Inserter<TopBookDb>,
    balance: Inserter<BalanceDb>,
    trade: Inserter<TradeDb>,
    open_order: Inserter<OpenOrderDb>,
    position: Inserter<PositionDb>,
    exposure: Inserter<ExposureDb>,
    indicator: Inserter<IndicatorDb>,
    strategy_log: Inserter<StrategyLogsDb>,
    transformer_log: Inserter<TransformerLogsDb>,
    public_trade: Inserter<PublicTradeDb>,
    latency: Inserter<LatencySnapshotRow>,
    counter: Inserter<CounterRow>,
    storage: Inserter<StorageDb>,
    evm_tx: Inserter<EvmTxDb>,
}

impl BackgroundInserters {
    fn new(client: &clickhouse::Client, period: Duration) -> Self {
        Self {
            top_book: client
                .inserter::<TopBookDb>("top_book")
                .with_period(Some(period)),
            balance: client
                .inserter::<BalanceDb>("balance")
                .with_period(Some(period)),
            trade: client
                .inserter::<TradeDb>("trade")
                .with_period(Some(period)),
            open_order: client
                .inserter::<OpenOrderDb>("open_order")
                .with_period(Some(period)),
            position: client
                .inserter::<PositionDb>("position")
                .with_period(Some(period)),
            exposure: client
                .inserter::<ExposureDb>("exposure")
                .with_period(Some(period)),
            indicator: client
                .inserter::<IndicatorDb>("indicator")
                .with_period(Some(period)),
            strategy_log: client
                .inserter::<StrategyLogsDb>("strategy_log")
                .with_period(Some(period)),
            transformer_log: client
                .inserter::<TransformerLogsDb>("transformer_log")
                .with_period(Some(period)),
            public_trade: client
                .inserter::<PublicTradeDb>("public_trade")
                .with_period(Some(period)),
            latency: client
                .inserter::<LatencySnapshotRow>("latency")
                .with_period(Some(period)),
            counter: client
                .inserter::<CounterRow>("counter_metric")
                .with_period(Some(period)),
            storage: client
                .inserter::<StorageDb>("storage")
                .with_period(Some(period)),
            evm_tx: client
                .inserter::<EvmTxDb>("evm_tx")
                .with_period(Some(period)),
        }
    }

    async fn insert(&mut self, record: Record) -> Result<()> {
        match record {
            Record::TopBook(v) => self.top_book.write(&v).await,
            Record::Balance(v) => self.balance.write(&v).await,
            Record::Trade(v) => self.trade.write(&v).await,
            Record::OpenOrder(v) => self.open_order.write(&v).await,
            Record::Position(v) => self.position.write(&v).await,
            Record::Exposure(v) => self.exposure.write(&v).await,
            Record::Indicator(v) => self.indicator.write(&v).await,
            Record::StrategyLog(v) => self.strategy_log.write(&v).await,
            Record::TransformerLog(v) => self.transformer_log.write(&v).await,
            Record::Latency(v) => self.latency.write(&v).await,
            Record::Counter(v) => self.counter.write(&v).await,
            Record::Storage(v) => self.storage.write(&v).await,
            Record::PublicTrade(v) => self.public_trade.write(&v).await,
            Record::EvmTx(v) => self.evm_tx.write(&v).await,
        }
        .map_err(|e| anyhow::anyhow!("ClickHouse write error: {}", e))
    }

    async fn insert_batch(&mut self, records: Vec<Record>) -> Result<()> {
        for record in records {
            self.insert(record).await?;
        }
        Ok(())
    }

    async fn commit(&mut self) -> Result<()> {
        let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10, r11, r12, r13, r14) = tokio::join!(
            self.top_book.commit(),
            self.balance.commit(),
            self.trade.commit(),
            self.open_order.commit(),
            self.position.commit(),
            self.exposure.commit(),
            self.indicator.commit(),
            self.strategy_log.commit(),
            self.transformer_log.commit(),
            self.public_trade.commit(),
            self.latency.commit(),
            self.counter.commit(),
            self.storage.commit(),
            self.evm_tx.commit(),
        );
        let results = [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10, r11, r12, r13, r14];

        // Collect errors
        for res in results {
            if let Err(e) = res {
                error!("ClickHouse commit error: {:?}", e);
                // We continue to check others, but we could return early.
                // Returning the first error found.
                return Err(anyhow::anyhow!("ClickHouse commit error: {}", e));
            }
        }
        Ok(())
    }
}

pub struct ClickHouseDb {
    sender: mpsc::Sender<DbMessage>,
    handles: Vec<JoinHandle<()>>,
}

impl ClickHouseDb {
    pub async fn try_new(config: ClickHouseConfig) -> Result<Self> {
        let mut client = clickhouse::Client::default().with_url(config.url);
        client = client.with_user(config.user);
        client = client.with_password(config.password);
        client = client.with_database(config.db_name);

        let client = client
            .with_option("allow_experimental_json_type", "1")
            .with_option("input_format_binary_read_json_as_string", "1")
            .with_option("output_format_binary_write_json_as_string", "1");

        let client = Arc::new(client);
        let migrate = Migration::new(client.clone());

        migrate.run().await?;

        let (tx, mut rx) = mpsc::channel(10000);
        let mut inserters = BackgroundInserters::new(&client, config.flush_interval);
        let flush_interval = config.flush_interval;

        let db_handle = tokio::spawn(async move {
            let mut ticker = interval(flush_interval);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if let Err(err) = inserters.commit().await {
                            error!("ClickHouse auto-flush error: {err:?}");
                        }
                    }
                    msg = rx.recv() => {
                        match msg {
                            Some(DbMessage::Insert(record)) => {
                                if let Err(err) = inserters.insert(record).await {
                                    error!("ClickHouse insert error: {err:?}");
                                }
                            }
                            Some(DbMessage::Batch(records)) => {
                                if let Err(err) = inserters.insert_batch(records).await {
                                    error!("ClickHouse batch insert error: {err:?}");
                                }
                            }
                            Some(DbMessage::Flush(reply)) => {
                                let res = inserters.commit().await;
                                let _ = reply.send(res);
                            }
                            None => break,
                        }
                    }
                }
            }
            // Final flush
            if let Err(err) = inserters.commit().await {
                error!("ClickHouse final flush error: {err:?}");
            }
        });

        let metrics_handle = Self::spawn_metrics_task(tx.clone(), config.metrics_interval);

        Ok(Self {
            sender: tx,
            handles: vec![db_handle, metrics_handle],
        })
    }

    pub async fn insert(&self, record: Record) -> Result<()> {
        self.sender
            .send(DbMessage::Insert(record))
            .await
            .map_err(|_| anyhow::anyhow!("Failed to send record to ClickHouse task"))
    }

    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(DbMessage::Flush(tx))
            .await
            .map_err(|_| anyhow::anyhow!("Failed to send flush request"))?;
        rx.await?
    }

    fn spawn_metrics_task(sender: mpsc::Sender<DbMessage>, period: Duration) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = interval(period);
            loop {
                ticker.tick().await;

                let latencies = get_all_nonempty_stats();
                reset_all_latencies();

                for latency in latencies {
                    if let Err(err) = sender
                        .send(DbMessage::Insert(Record::Latency(latency)))
                        .await
                    {
                        error!("Failed to send metrics: {:?}", err);
                    }
                }

                let counters = get_all_counters();

                let now = Utc::now();
                for counter in counters {
                    let row = CounterRow {
                        recorded_at: now,
                        name: counter.name.to_string(),
                        count: counter.count,
                    };
                    if let Err(err) = sender.send(DbMessage::Insert(Record::Counter(row))).await {
                        error!("Failed to send metrics: {:?}", err);
                    }
                }
            }
        })
    }

    pub async fn stop(&self) {
        if let Err(err) = self.flush().await {
            error!("final ClickHouse flush failed: {err:?}");
        }
        for handle in &self.handles {
            handle.abort();
        }
    }
}

#[async_trait::async_trait]
impl AnalyticRepository for ClickHouseDb {
    async fn batch_insert(&self, records: Vec<Record>) -> Result<()> {
        self.sender
            .send(DbMessage::Batch(records))
            .await
            .map_err(|_| anyhow::anyhow!("Failed to send batch to ClickHouse task"))
    }
}
