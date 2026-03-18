use crate::runtime::Runtime;
use anyhow::{anyhow, Result};
use rengine_metrics::latencies::record_latency;
use rengine_types::{ExecutionRequest, ExecutionRequestsWithLogs, StrategyConfiguration};
use smol_str::SmolStr;
use std::time::Instant;
use wasmtime::component::{bindgen, Component};

bindgen!({world: "strategy", path: "../strategy_api/strategy.wit",});

pub trait StrategyRuntime {
    fn instantiate_strategy(&mut self, wasm: &[u8]) -> Result<Strategy>;
    fn subscriptions(&mut self, strategy: &Strategy) -> Result<StrategyConfiguration>;
    fn execute(
        &mut self,
        strategy: &Strategy,
        state: &[u8],
        latency_ids: Option<&LatencyIds>,
    ) -> Result<(Vec<u8>, ExecutionRequestsWithLogs)>;
    fn instantiate_and_run(&mut self, wasm: &[u8]) -> Result<ExecutionRequestsWithLogs>;
}

/// Precomputed latency IDs for a strategy/transformer to avoid allocations on the hot path
pub struct LatencyIds {
    pub wasm_exec: SmolStr,
    pub state_deserialize: SmolStr,
}

impl LatencyIds {
    pub fn new(prefix: &str, id: &str) -> Self {
        Self {
            wasm_exec: format!("{}_{}_wasm_exec", prefix, id).into(),
            state_deserialize: format!("{}_{}_state_deserialize", prefix, id).into(),
        }
    }
}

impl StrategyRuntime for Runtime {
    fn instantiate_strategy(&mut self, wasm: &[u8]) -> Result<Strategy> {
        let component = unsafe {
            Component::deserialize(&self.engine, wasm)
                .or_else(|_| Component::new(&self.engine, wasm))?
        };
        let strategy = Strategy::instantiate(&mut self.store, &component, &self.linker)?;
        Ok(strategy)
    }

    fn subscriptions(&mut self, strategy: &Strategy) -> Result<StrategyConfiguration> {
        let results = strategy
            .call_init(&mut self.store)
            .map_err(|err| anyhow!(err))?;

        borsh::from_slice(&results).map_err(|err| anyhow!(err))
    }

    fn execute(
        &mut self,
        strategy: &Strategy,
        state: &[u8],
        latency_ids: Option<&LatencyIds>,
    ) -> Result<(Vec<u8>, ExecutionRequestsWithLogs)> {
        let exec_start = Instant::now();
        let (new_state, requests_bytes) = strategy
            .call_exec(&mut self.store, state)
            .map_err(|err| anyhow!(err))?
            .map_err(|err| anyhow!(err))?;
        if let Some(ids) = latency_ids {
            record_latency(ids.wasm_exec.clone(), exec_start);
        }

        let deserialize_start = Instant::now();
        let requests: Vec<ExecutionRequest> =
            borsh::from_slice(&requests_bytes).map_err(|err| anyhow!(err))?;
        if let Some(ids) = latency_ids {
            record_latency(ids.state_deserialize.clone(), deserialize_start);
        }

        Ok((
            new_state,
            ExecutionRequestsWithLogs {
                requests,
                logs: self.take_logs(),
            },
        ))
    }

    fn instantiate_and_run(&mut self, wasm: &[u8]) -> Result<ExecutionRequestsWithLogs> {
        let strategy = self.instantiate_strategy(wasm)?;

        self.execute(&strategy, &[], None).map(|(_, reqs)| reqs)
    }
}
