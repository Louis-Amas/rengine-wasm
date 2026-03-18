use crate::runtime::Runtime;
use alloy::sol_types::{sol_data::Array, SolType};
use anyhow::{anyhow, Result};
use evm_types::{Call3, Result as MulticallResult};
use rengine_types::{evm::MulticallPluginConfig, Action};
use wasmtime::component::{bindgen, Component};

bindgen!({world: "multicall", path: "../evm_multicall_api/multicall.wit",});

pub trait MulticallRuntime {
    fn instantiate_multicall_reader(&mut self, wasm: &[u8]) -> Result<Multicall>;
    fn execute_multicall_reader_init(&mut self, reader: &Multicall) -> Result<Vec<u8>>;
    fn execute_multicall_reader_config(
        &mut self,
        reader: &Multicall,
    ) -> Result<MulticallPluginConfig>;
    fn execute_multicall_reader_requests(&mut self, reader: &Multicall) -> Result<Vec<Call3>>;
    fn execute_multicall_reader_handle(
        &mut self,
        reader: &Multicall,
        state: &[u8],
        results: &[MulticallResult],
    ) -> Result<(Vec<u8>, Vec<Action>)>;
}

impl MulticallRuntime for Runtime {
    fn instantiate_multicall_reader(&mut self, wasm: &[u8]) -> Result<Multicall> {
        let component = unsafe { Component::deserialize(&self.engine, wasm)? };
        Multicall::instantiate(&mut self.store, &component, &self.linker)
    }

    fn execute_multicall_reader_init(&mut self, reader: &Multicall) -> Result<Vec<u8>> {
        reader.call_init(&mut self.store)
    }

    fn execute_multicall_reader_config(
        &mut self,
        reader: &Multicall,
    ) -> Result<MulticallPluginConfig> {
        let config = reader.call_config(&mut self.store)?;

        borsh::from_slice(&config)
            .map_err(|err| anyhow!("couldn't execute multical reader config {err:?}"))
    }

    fn execute_multicall_reader_requests(&mut self, reader: &Multicall) -> Result<Vec<Call3>> {
        let results = reader
            .call_requests(&mut self.store)
            .map_err(|err| anyhow!(err))?;

        let calls: Vec<Call3> = <Array<Call3> as SolType>::abi_decode(&results)
            .map_err(|err| anyhow!("couldn't execute multical reader requests {err:?}"))?;

        Ok(calls)
    }

    fn execute_multicall_reader_handle(
        &mut self,
        reader: &Multicall,
        state: &[u8],
        results: &[MulticallResult],
    ) -> Result<(Vec<u8>, Vec<Action>)> {
        let encoded = <Array<MulticallResult> as SolType>::abi_encode(&results);
        let (new_state, results) = reader
            .call_handle(&mut self.store, state, &encoded)?
            .map_err(|err| anyhow!(err))?;

        let actions = borsh::from_slice(&results)?;
        Ok((new_state, actions))
    }
}
