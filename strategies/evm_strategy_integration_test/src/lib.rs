use alloy::{
    primitives::{address, Address, U256},
    sol_types::{SolCall, SolValue},
};
use anyhow::Result;
use evm_types::{erc20::ERC20Mock, EvmTxRequest};
use rengine_types::{EvmAccount, ExecutionRequest, StateUpdateKey, StrategyConfiguration, Venue};
use std::{collections::HashSet, time::Duration};
use strategy_api::{bindings::export, impl_guest_from_plugin, Plugin};

struct MyPlugin;

#[derive(Default, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct State;

impl Plugin for MyPlugin {
    type State = State;

    fn init() -> StrategyConfiguration {
        let mut set = HashSet::new();
        set.insert(StateUpdateKey::UtcTimer {
            interval: Duration::from_secs(10),
        });
        StrategyConfiguration {
            triggers_keys: set,
            cooldown: None,
        }
    }

    fn execute(state: Self::State) -> Result<(Self::State, Vec<ExecutionRequest>), String> {
        let venue: Venue = "anvil".into();
        let account = EvmAccount {
            venue,
            account_id: "default".into(),
        };

        // Create an approve call
        let approve_call = ERC20Mock::approveCall {
            spender: Address::from([0x01; 20]),
            amount: U256::from(1000),
        };
        let call_data = approve_call.abi_encode();

        // MockERC20 address on Anvil (Nonce 1 of default account)
        let target = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");

        let tx_request = EvmTxRequest {
            to: target,
            value: U256::ZERO,
            data: call_data.into(),
        };
        let data = tx_request.abi_encode();

        // Create a transfer call
        let transfer_call = ERC20Mock::transferCall {
            to: Address::from([0x02; 20]),
            amount: U256::from(100),
        };
        let transfer_call_data = transfer_call.abi_encode();

        let transfer_tx_request = EvmTxRequest {
            to: target,
            value: U256::ZERO,
            data: transfer_call_data.into(),
        };
        let transfer_data = transfer_tx_request.abi_encode();

        Ok((
            state,
            vec![
                ExecutionRequest::EvmTx((account.clone(), data.into())),
                ExecutionRequest::EvmTx((account, transfer_data.into())),
            ],
        ))
    }
}

impl_guest_from_plugin!(MyPlugin, "evm_strategy_integration_test");

export!(MyPlugin with_types_in strategy_api::bindings);
