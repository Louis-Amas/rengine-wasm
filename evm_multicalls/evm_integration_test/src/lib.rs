use alloy::{
    primitives::{address, U256},
    sol_types::{SolCall, SolValue},
};
use anyhow::Result;
use evm_multicall_api::{
    bindings::export, impl_guest_from_multi_callplugin, Call3, MulticallPlugin, MulticallResult,
};
use evm_types::erc20::ERC20Mock::balanceOfCall;
use rengine_types::{evm::MulticallPluginConfig, Action};
use rust_decimal::Decimal;
use std::str::FromStr;

struct TestMulticall;

#[derive(Default, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct State {}

impl MulticallPlugin for TestMulticall {
    type State = State;

    fn config() -> MulticallPluginConfig {
        MulticallPluginConfig { every_x_block: 1 }
    }

    fn requests() -> Vec<Call3> {
        let balance = balanceOfCall {
            account: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        };

        // MockERC20 address on Anvil (Nonce 1 of default account)
        let target = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");

        vec![
            Call3 {
                target,
                allowFailure: false,
                callData: balance.abi_encode().into(),
            },
            Call3 {
                target,
                allowFailure: false,
                callData: balance.abi_encode().into(),
            },
        ]
    }

    fn handle(
        state: Self::State,
        results: Vec<MulticallResult>,
    ) -> Result<(Self::State, Vec<rengine_types::Action>), String> {
        let first = results.into_iter().next().unwrap();

        let balance = U256::abi_decode(&first.returnData).map_err(|err| err.to_string())?;

        // Convert balance (wei) to decimal string with 18 decimals
        let decimals = U256::from(10u128.pow(18));
        let integer = balance / decimals;
        let remainder = balance % decimals;
        let mut rem_s = remainder.to_string();
        if rem_s.len() < 18 {
            rem_s = format!("{:0>18}", rem_s);
        }
        let balance_decimal = Decimal::from_str(&format!("{}.{}", integer, rem_s)).unwrap();

        let action = Action::SetIndicator("test".into(), balance_decimal);
        let storage_action = Action::SetStorage("balance".into(), balance.abi_encode());

        Ok((state, vec![action, storage_action]))
    }
}

impl_guest_from_multi_callplugin!(TestMulticall);

export!(TestMulticall with_types_in evm_multicall_api::bindings);
