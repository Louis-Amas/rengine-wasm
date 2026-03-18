use crate::runtime::Runtime;
use anyhow::Result;
use wasmtime::component::{bindgen, Component};

bindgen!({world: "evm-logs", path: "../evm_logs_api/evm_logs.wit",});

pub trait EvmLogsRuntime {
    fn instantiate_evm_logs(&mut self, wasm: &[u8]) -> Result<EvmLogs>;
    fn execute_evm_logs_init(&mut self, logs: &EvmLogs) -> Result<(Vec<u8>, Vec<u8>)>;
    fn execute_evm_logs_handle(
        &mut self,
        logs: &EvmLogs,
        state: &[u8],
        log: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>)>;
}

impl EvmLogsRuntime for Runtime {
    fn instantiate_evm_logs(&mut self, wasm: &[u8]) -> Result<EvmLogs> {
        let component = unsafe {
            Component::deserialize(&self.engine, wasm)
                .or_else(|_| Component::new(&self.engine, wasm))?
        };
        let evm_logs = EvmLogs::instantiate(&mut self.store, &component, &self.linker)?;
        Ok(evm_logs)
    }

    fn execute_evm_logs_init(&mut self, logs: &EvmLogs) -> Result<(Vec<u8>, Vec<u8>)> {
        logs.call_init(&mut self.store)
    }

    fn execute_evm_logs_handle(
        &mut self,
        logs: &EvmLogs,
        state: &[u8],
        log: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        logs.call_handle_log_message(&mut self.store, state, log)
    }
}
