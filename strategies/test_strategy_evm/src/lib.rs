use alloy::sol_types::{SolCall, SolValue};
use anyhow::Result;
use evm_types::{erc20::ERC20Mock, EvmTxRequest};
use rengine_types::{EvmAccount, ExecutionRequest, StrategyConfiguration, Venue};
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
        let venue: Venue = "venue".into();
        let account = EvmAccount {
            venue,
            account_id: "test".into(),
        };

        // Create an approve call
        let approve_call = ERC20Mock::approveCall {
            spender: alloy::primitives::Address::from([0x01; 20]),
            amount: alloy::primitives::U256::from(1000),
        };
        let call_data = approve_call.abi_encode();

        let tx_request = EvmTxRequest {
            to: alloy::primitives::Address::from([0x02; 20]),
            value: alloy::primitives::U256::ZERO,
            data: call_data.into(),
        };
        let data = tx_request.abi_encode();

        Ok((state, vec![ExecutionRequest::EvmTx((account, data.into()))]))
    }
}

impl_guest_from_plugin!(MyPlugin, "test_strategy_evm");

export!(MyPlugin with_types_in strategy_api::bindings);
