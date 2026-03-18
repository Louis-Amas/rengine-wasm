use crate::{
    impl_imports_for,
    strategy::{Strategy, StrategyImports},
    types::WasmStateWrapper,
};
use anyhow::Result;
use parking_lot::RwLock;
use rengine_types::{PublicTrade, State, VenueBookKey};
use std::{collections::HashMap, sync::Arc};
use wasmtime::{
    component::{HasSelf, Linker},
    Config, Engine, Store,
};
use wasmtime_wasi::{
    p2::add_to_linker_sync, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};

pub struct WasmState {
    pub inner: WasmStateWrapper,
    pub wasi_ctx: WasiCtx,
    pub resource_table: ResourceTable,
}

impl_imports_for!(StrategyImports, WasmState);

// INFO: this is safe because we run strategies on the same thread
unsafe impl Send for WasmState {}
unsafe impl Sync for WasmState {}

impl WasiView for WasmState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.resource_table,
        }
    }
}

/// Configuration for WASM execution resource limits.
#[derive(Debug, Clone, Default)]
pub struct FuelConfig {
    /// Maximum fuel units per execution (None = unlimited).
    pub max_fuel: Option<u64>,
    /// Whether to enable epoch-based interruption.
    pub epoch_interruption: bool,
}

pub struct Runtime {
    pub engine: Engine,
    pub linker: Linker<WasmState>,
    pub store: Store<WasmState>,
    fuel_config: FuelConfig,
}

impl Runtime {
    // FIXME: Known limitation is shared between runtime. The use of it should be extra careful.
    pub fn new(state: Arc<RwLock<State>>) -> Result<Self> {
        Self::new_with_fuel(state, FuelConfig::default())
    }

    pub fn new_with_fuel(state: Arc<RwLock<State>>, fuel_config: FuelConfig) -> Result<Self> {
        let mut config = Config::new();
        config.async_support(false);
        config.wasm_component_model(true);
        config.debug_info(false);

        if fuel_config.max_fuel.is_some() {
            config.consume_fuel(true);
        }
        if fuel_config.epoch_interruption {
            config.epoch_interruption(true);
        }

        let engine = Engine::new(&config)?;

        let mut builder = WasiCtxBuilder::new();

        let inner = WasmStateWrapper {
            state,
            logs: Default::default(),
            aggregated_trade_flows: Default::default(),
        };

        let wasi_ctx = builder.build();
        let resource_table = ResourceTable::new();

        let mut store = Store::new(
            &engine,
            WasmState {
                inner,
                wasi_ctx,
                resource_table,
            },
        );

        if let Some(fuel) = fuel_config.max_fuel {
            store.set_fuel(fuel)?;
        }
        if fuel_config.epoch_interruption {
            store.epoch_deadline_trap();
        }

        let mut linker: Linker<WasmState> = Linker::new(&engine);

        add_to_linker_sync(&mut linker)?;
        Strategy::add_to_linker::<_, HasSelf<_>>(&mut linker, |state: &mut _| state)?;

        Ok(Self {
            engine,
            store,
            linker,
            fuel_config,
        })
    }

    pub fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().inner.logs)
    }

    /// Set the aggregated trade flows for the current execution cycle
    pub fn set_aggregated_trade_flows(
        &mut self,
        trade_flows: HashMap<VenueBookKey, Vec<PublicTrade>>,
    ) {
        self.store.data_mut().inner.aggregated_trade_flows = trade_flows;
    }

    /// Clear the aggregated trade flows after execution
    pub fn clear_aggregated_trade_flows(&mut self) {
        self.store.data_mut().inner.aggregated_trade_flows.clear();
    }

    /// Refuel the store before each execution (if fuel metering is enabled).
    pub fn refuel(&mut self) -> Result<()> {
        if let Some(max_fuel) = self.fuel_config.max_fuel {
            let remaining = self.store.get_fuel().unwrap_or(0);
            if remaining < max_fuel {
                self.store.set_fuel(max_fuel)?;
            }
        }
        Ok(())
    }

    /// Get remaining fuel (returns None if fuel metering is disabled).
    pub fn remaining_fuel(&self) -> Option<u64> {
        if self.fuel_config.max_fuel.is_some() {
            self.store.get_fuel().ok()
        } else {
            None
        }
    }

    /// Get a reference to the engine (needed for epoch incrementing).
    pub fn engine_ref(&self) -> &Engine {
        &self.engine
    }
}
