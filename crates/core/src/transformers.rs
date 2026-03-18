use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeZone, Utc};
use parking_lot::RwLock as ParkingLotRwLock;
use rengine_interfaces::db::TransformerRepository;
use rengine_metrics::latencies::record_latency;
use rengine_types::{
    db::TransformerDb, ExecutionRequestsWithLogs, PublicTrade, State, StateUpdateKey,
    StrategyConfiguration, TransformerExecutionResult, TransformerId, VenueBookKey,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tracing::warn;
use wasm_runtime::{
    strategy::{LatencyIds, Strategy, StrategyRuntime},
    Runtime,
};

struct TransformerWrapper {
    strategy: Strategy,
    keys: HashSet<StateUpdateKey>,
    enabled: bool,
    interval: Option<Duration>,
    next_execution: Option<Instant>,
    utc_interval: Option<Duration>,
    next_utc_execution: Option<DateTime<Utc>>,
    cooldown: Option<Duration>,
    last_execution: Option<Instant>,
    state: Vec<u8>,
    latency_ids: LatencyIds,
}

impl TransformerWrapper {
    fn new(
        id: &TransformerId,
        strategy: Strategy,
        config: StrategyConfiguration,
        enabled: bool,
    ) -> Self {
        let mut interval = None;
        let mut utc_interval = None;
        for key in &config.triggers_keys {
            match key {
                StateUpdateKey::Timer { interval: i } => {
                    interval = Some(*i);
                }
                StateUpdateKey::UtcTimer { interval: i } => {
                    utc_interval = Some(*i);
                }
                _ => {}
            }
        }

        Self {
            strategy,
            keys: config.triggers_keys,
            enabled,
            interval,
            next_execution: None,
            utc_interval,
            next_utc_execution: None,
            cooldown: config.cooldown,
            last_execution: None,
            state: Vec::new(),
            latency_ids: LatencyIds::new("transformer", id),
        }
    }

    fn execute_with_filter(
        &mut self,
        updated_keys: &HashSet<StateUpdateKey>,
        runtime: &mut Runtime,
        now: Instant,
        utc_now: DateTime<Utc>,
    ) -> Result<ExecutionRequestsWithLogs> {
        if let Some(cooldown) = self.cooldown {
            if let Some(last_exec) = self.last_execution {
                if now < last_exec + cooldown {
                    return Ok(Default::default());
                }
            }
        }

        let mut should_run = !self.keys.is_disjoint(updated_keys);

        if let (Some(interval), Some(next_exec)) = (self.interval, self.next_execution) {
            if now >= next_exec {
                should_run = true;
                self.next_execution = Some(now + interval);
            }
        } else if let Some(interval) = self.interval {
            // First run or reset
            if self.next_execution.is_none() {
                self.next_execution = Some(now + interval);
            }
        }

        if let Some(interval) = self.utc_interval {
            let interval_chrono = chrono::Duration::from_std(interval)?;
            if let Some(next_exec) = self.next_utc_execution {
                if utc_now >= next_exec {
                    should_run = true;
                    let mut next = next_exec + interval_chrono;
                    // Skip missed intervals
                    while next <= utc_now {
                        next += interval_chrono;
                    }
                    self.next_utc_execution = Some(next);
                }
            } else {
                let now_ts =
                    utc_now.timestamp() * 1_000_000_000 + utc_now.timestamp_subsec_nanos() as i64;
                let interval_nanos = interval_chrono
                    .num_nanoseconds()
                    .ok_or_else(|| anyhow!("Interval too large"))?;

                if interval_nanos > 0 {
                    let remainder = now_ts % interval_nanos;
                    let next_ts = now_ts - remainder + interval_nanos;
                    let next_ts_secs = next_ts / 1_000_000_000;
                    let next_ts_nanos = (next_ts % 1_000_000_000) as u32;
                    self.next_utc_execution =
                        Utc.timestamp_opt(next_ts_secs, next_ts_nanos).single();
                }
            }
        }

        if should_run {
            runtime
                .execute(&self.strategy, &self.state, Some(&self.latency_ids))
                .map(|(new_state, reqs)| {
                    self.state = new_state;
                    self.last_execution = Some(now);
                    reqs
                })
        } else {
            Ok(Default::default())
        }
    }
}

struct TransformersHandlerInner {
    transformers: HashMap<TransformerId, TransformerWrapper>,
    runtime: Runtime,
    db: Arc<dyn TransformerRepository>,
}

#[derive(Clone)]
pub struct TransformersHandler {
    inner: Arc<RwLock<TransformersHandlerInner>>,
}

impl TransformersHandler {
    pub(crate) async fn new(
        state: Arc<ParkingLotRwLock<State>>,
        db: Arc<dyn TransformerRepository>,
    ) -> Result<Self> {
        let handler = Self {
            inner: Arc::new(RwLock::new(TransformersHandlerInner {
                transformers: Default::default(),
                runtime: Runtime::new(state)?,
                db: db.clone(),
            })),
        };
        let transformers = db.list_transformers().await?;

        for transformer in transformers {
            if let Err(err) = handler
                .add(
                    transformer.transformer_name.into(),
                    &transformer.wasm,
                    transformer.enabled,
                )
                .await
            {
                warn!(?err, "couldn't add transformer");
            }
        }

        Ok(handler)
    }

    pub async fn add(&self, id: TransformerId, wasm: &[u8], enabled: bool) -> Result<()> {
        let mut lock = self.inner.write().await;
        let strategy = lock.runtime.instantiate_strategy(wasm)?;

        let config = lock.runtime.subscriptions(&strategy)?;
        let transformer = TransformerWrapper::new(&id, strategy, config, enabled);

        lock.db
            .add_transformer(TransformerDb {
                enabled,
                wasm: wasm.to_vec(),
                transformer_name: id.to_string(),
            })
            .await?;

        lock.transformers.insert(id, transformer);

        Ok(())
    }

    pub async fn set_enabled(&self, id: TransformerId, enabled: bool) -> Result<()> {
        let mut lock = self.inner.write().await;

        lock.db.set_transformer_enable(&id, enabled).await?;

        let transformer = lock
            .transformers
            .get_mut(&id)
            .ok_or_else(|| anyhow!("missing transformer with id {id}"))?;

        transformer.enabled = enabled;

        Ok(())
    }

    pub(crate) async fn execute_transformers(
        &self,
        keys: &HashSet<StateUpdateKey>,
        now: Instant,
        aggregated_trades: HashMap<VenueBookKey, Vec<PublicTrade>>,
    ) -> Vec<TransformerExecutionResult> {
        let mut results = vec![];
        let time = Utc::now();
        let TransformersHandlerInner {
            transformers,
            runtime,
            db: _,
        } = &mut *self.inner.write().await;

        // Set the aggregated trades for all transformer executions (takes ownership)
        runtime.set_aggregated_trade_flows(aggregated_trades);

        for (transformer_id, transformer) in transformers {
            if !transformer.enabled {
                continue;
            }

            let transformer_start = Instant::now();
            match transformer.execute_with_filter(keys, runtime, now, time) {
                Ok(result) => {
                    record_latency(transformer.latency_ids.wasm_exec.clone(), transformer_start);

                    let result = TransformerExecutionResult {
                        emitted_at: time,
                        transformer_id: transformer_id.clone(),
                        execution_result: result,
                    };

                    results.push(result);
                }
                Err(err) => warn!("error in transformer {err}"),
            }
        }

        // Clear after all executions
        runtime.clear_aggregated_trade_flows();

        results
    }

    pub async fn instantiate_and_execute(&self, wasm: &[u8]) -> Result<ExecutionRequestsWithLogs> {
        self.inner.write().await.runtime.instantiate_and_run(wasm)
    }
}
