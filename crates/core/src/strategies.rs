use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeZone, Utc};
use parking_lot::RwLock as ParkingLotRwLock;
use rengine_interfaces::db::StrategyRepository;
use rengine_metrics::latencies::record_latency;
use rengine_types::{
    db::StrategyDb, ExecutionRequestsWithLogs, State, StateUpdateKey, StrategyConfiguration,
    StrategyExecutionResult, StrategyId,
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

struct StrategyWrapper {
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

impl StrategyWrapper {
    fn new(
        id: &StrategyId,
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
            latency_ids: LatencyIds::new("strategy", id),
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

struct StrategiesHandlerInner {
    strategies: HashMap<StrategyId, StrategyWrapper>,
    runtime: Runtime,
    db: Arc<dyn StrategyRepository>,
}

#[derive(Clone)]
pub struct StrategiesHandler {
    inner: Arc<RwLock<StrategiesHandlerInner>>,
}

impl StrategiesHandler {
    pub(crate) async fn new(
        state: Arc<ParkingLotRwLock<State>>,
        db: Arc<dyn StrategyRepository>,
    ) -> Result<Self> {
        let handler = Self {
            inner: Arc::new(RwLock::new(StrategiesHandlerInner {
                strategies: Default::default(),
                runtime: Runtime::new(state)?,
                db: db.clone(),
            })),
        };
        let strategies = db.list_strategies().await?;

        for strategy in strategies {
            if let Err(err) = handler
                .add(
                    strategy.strategy_name.into(),
                    &strategy.wasm,
                    strategy.enabled,
                )
                .await
            {
                warn!(?err, "couldn't add strategy");
            }
        }

        Ok(handler)
    }

    pub async fn add(&self, id: StrategyId, wasm: &[u8], enabled: bool) -> Result<()> {
        let mut lock = self.inner.write().await;
        let strategy = lock.runtime.instantiate_strategy(wasm)?;

        let config = lock.runtime.subscriptions(&strategy)?;
        let strategy = StrategyWrapper::new(&id, strategy, config, enabled);

        lock.db
            .add_strategy(StrategyDb {
                enabled,
                wasm: wasm.to_vec(),
                strategy_name: id.to_string(),
            })
            .await?;

        lock.strategies.insert(id, strategy);

        Ok(())
    }

    pub async fn set_enabled(&self, id: StrategyId, enabled: bool) -> Result<()> {
        let mut lock = self.inner.write().await;

        lock.db.set_enable(&id, enabled).await?;

        let strategy = lock
            .strategies
            .get_mut(&id)
            .ok_or_else(|| anyhow!("missing strategy with id {id}"))?;

        strategy.enabled = enabled;

        Ok(())
    }

    pub(crate) async fn execute_strategies(
        &self,
        keys: HashSet<StateUpdateKey>,
        now: Instant,
    ) -> Vec<StrategyExecutionResult> {
        let mut results = vec![];
        let time = Utc::now();
        let StrategiesHandlerInner {
            strategies,
            runtime,
            db: _,
        } = &mut *self.inner.write().await;

        for (strategy_id, strategy) in strategies {
            if !strategy.enabled {
                continue;
            }

            let strategy_start = Instant::now();
            match strategy.execute_with_filter(&keys, runtime, now, time) {
                Ok(result) => {
                    record_latency(strategy.latency_ids.wasm_exec.clone(), strategy_start);

                    let result = StrategyExecutionResult {
                        emitted_at: time,
                        strategy_id: strategy_id.clone(),
                        execution_result: result,
                    };

                    results.push(result);
                }
                Err(err) => warn!("error in strategy {err}"),
            }
        }

        results
    }

    pub async fn instantiate_and_execute(&self, wasm: &[u8]) -> Result<ExecutionRequestsWithLogs> {
        self.inner.write().await.runtime.instantiate_and_run(wasm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rengine_interfaces::db::MockStrategyRepository;

    const TEST_STRATEGY_BYTES: &[u8] =
        include_bytes!("../../../strategies-wasm/test_strategy_state.cwasm");

    // Helper to create a default State behind ParkingLot RwLock
    fn mk_state() -> Arc<ParkingLotRwLock<State>> {
        Arc::new(ParkingLotRwLock::new(State::default()))
    }

    #[tokio::test]
    async fn test_add_then_toggle_updates_db_and_memory() {
        // Arrange
        let state = mk_state();

        let id: StrategyId = "dummy-strategy".into();
        let enabled_initial = true;
        let enabled_after = false; // we'll toggle to this

        // Mock repo expectations
        let mut mock = MockStrategyRepository::new();
        mock.expect_list_strategies()
            .returning(|| Box::pin(async { Ok(Vec::new()) }));
        // Expect the add() persistence with exact payload
        mock.expect_add_strategy()
            .withf({
                let id = id.clone();
                move |s: &StrategyDb| {
                    s.enabled == enabled_initial
                        && s.strategy_name == id
                        && s.wasm == TEST_STRATEGY_BYTES.to_vec()
                }
            })
            .times(1)
            .returning(|_s| Box::pin(async { Ok(()) }));

        // Expect a single toggle to `enabled_after`
        mock.expect_set_enable()
            .withf(move |_, toggle| *toggle == enabled_after)
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(()) }));

        // No list calls expected
        mock.expect_list_strategies().times(0);

        let repo: Arc<dyn StrategyRepository> = Arc::new(mock);
        let handler = StrategiesHandler::new(state, repo).await.expect("handler");

        // Act 1: add
        handler
            .add(id.clone(), TEST_STRATEGY_BYTES, enabled_initial)
            .await
            .unwrap();

        // Act 2: toggle
        handler
            .set_enabled(id.clone(), enabled_after)
            .await
            .unwrap();

        // Assert: in-memory state reflects toggle and keys are preserved
        let guard = handler.inner.read().await;
        let inner = &*guard;
        let sw = inner.strategies.get(&id).unwrap();

        assert_eq!(
            sw.enabled, enabled_after,
            "enabled flag should update after toggle()"
        );
    }
}

// #[cfg(test)]
// mod test {
//     use crate::strategies::StrategyWrapper;
//     use parking_lot::RwLock;
//     use rengine_types::{BalanceKey, State, StateUpdateKey};
//     use std::{collections::HashSet, sync::Arc};
//     use wasm_runtime::Runtime;
//
//     const STRATEGY_BYTES: &[u8] = include_bytes!("../../../strategies-wasm/simple_strategy.wasm");
//
//     #[test]
//     fn test_filter_strategy() {
//         let state: Arc<RwLock<State>> = Default::default();
//         let mut runtime = Runtime::new(state).unwrap();
//         let strategy = runtime.instantiate_strategy(STRATEGY_BYTES).unwrap();
//
//         let mut set = HashSet::new();
//         set.insert(StateUpdateKey::SetBalance(BalanceKey {
//             venue: "test".into(),
//             symbol: "eth".into(),
//         }));
//
//         let strat = StrategyWrapper {
//             keys: set.clone(),
//             strategy,
//         };
//
//         let result = strat.execute_with_filter(&set, &mut runtime).unwrap();
//         assert!(!result.is_empty());
//
//         let mut set = HashSet::new();
//         set.insert(StateUpdateKey::SetIndicator("test".into()));
//
//         let result = strat.execute_with_filter(&set, &mut runtime).unwrap();
//         assert!(result.is_empty());
//     }
// }
