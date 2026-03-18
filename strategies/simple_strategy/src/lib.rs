use anyhow::Result;
use rengine_types::{ExecutionRequest, StrategyConfiguration};
use strategy_api::{bindings::export, impl_guest_from_plugin, Plugin};

struct MyPlugin;

#[derive(Default, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct State;

impl Plugin for MyPlugin {
    type State = State;

    fn init() -> StrategyConfiguration {
        StrategyConfiguration {
            triggers_keys: <_>::default(),
            cooldown: None,
        }
    }

    fn execute(state: Self::State) -> Result<(Self::State, Vec<ExecutionRequest>), String> {
        Ok((state, vec![ExecutionRequest::Nothing]))
    }
}

impl_guest_from_plugin!(MyPlugin, "simple_strategy");

export!(MyPlugin with_types_in strategy_api::bindings);
