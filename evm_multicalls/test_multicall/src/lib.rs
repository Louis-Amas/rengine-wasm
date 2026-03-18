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
            account: address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        };

        vec![
            Call3 {
                target: address!("0xe7f1725e7734ce288f8367e1bb143e90bb3f0512"),
                allowFailure: false,
                callData: balance.abi_encode().into(),
            },
            Call3 {
                target: address!("0x1f9090aae28b8a3dceadf281b0f12828e676c326"),
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

        Ok((state, vec![action]))
    }
}

impl_guest_from_multi_callplugin!(TestMulticall);

export!(TestMulticall with_types_in evm_multicall_api::bindings);
