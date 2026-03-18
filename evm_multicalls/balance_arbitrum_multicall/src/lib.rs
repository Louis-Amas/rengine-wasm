use alloy::{
    primitives::{address, Address, U256},
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

const USDC_ARBITRUM: Address = address!("0xaf88d065e77c8cc2239327c5edb3a432268e5831");

struct BalanceUsdcArbitrumMulticall;

#[derive(Default, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct State {}

impl MulticallPlugin for BalanceUsdcArbitrumMulticall {
    type State = State;

    fn config() -> MulticallPluginConfig {
        MulticallPluginConfig { every_x_block: 100 }
    }

    fn requests() -> Vec<Call3> {
        let balance = balanceOfCall {
            account: address!("0x648Ff8f5699702d52Dae6b356EFAe7CA800e3327"),
        };

        vec![Call3 {
            target: USDC_ARBITRUM,
            allowFailure: false,
            callData: balance.abi_encode().into(),
        }]
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

impl_guest_from_multi_callplugin!(BalanceUsdcArbitrumMulticall);

export!(BalanceUsdcArbitrumMulticall with_types_in evm_multicall_api::bindings);
