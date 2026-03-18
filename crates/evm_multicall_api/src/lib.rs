use anyhow::Result;
pub use evm_types::{Call3, Result as MulticallResult};
use rengine_types::{evm::MulticallPluginConfig, Action, Decimal, OrderInfo, TopBookUpdate};
use std::{collections::HashMap, fmt::Arguments};

pub mod bindings {
    use wit_bindgen::generate;
    generate!({path: "multicall.wit", pub_export_macro: true, export_macro_name: "export", });
}

pub fn get_indicator(key: &str) -> Result<Decimal, String> {
    let value = bindings::indicator(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_balance(key: &str) -> Result<Decimal, String> {
    let value = bindings::balance(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_book(key: &str) -> Result<TopBookUpdate, String> {
    let value = bindings::book(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_open_orders(key: &str) -> Result<HashMap<String, OrderInfo>, String> {
    let value = bindings::open_orders(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_perp_position(key: &str) -> Result<Decimal, String> {
    let value = bindings::perp_positions(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_spot_exposure(key: &str) -> Result<Decimal, String> {
    let value = bindings::spot_exposure(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn trace(args: Arguments) {
    bindings::trace(&args.to_string());
}

// Now define a macro to use like printf!
#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        $crate::trace(format_args!($($arg)*))
    };
}

pub trait MulticallPlugin {
    type State: borsh::BorshSerialize + borsh::BorshDeserialize + Default;

    fn init() -> Self::State {
        Self::State::default()
    }
    fn config() -> MulticallPluginConfig;
    fn requests() -> Vec<Call3>;
    fn handle(
        state: Self::State,
        results: Vec<MulticallResult>,
    ) -> Result<(Self::State, Vec<Action>), String>;
}

#[macro_export]
macro_rules! impl_guest_from_multi_callplugin {
    ($plugin_type:ty) => {
        impl $crate::bindings::Guest for $plugin_type {
            fn init() -> Vec<u8> {
                let state = <$plugin_type as $crate::MulticallPlugin>::init();
                borsh::to_vec(&state).unwrap()
            }

            fn config() -> Vec<u8> {
                use rengine_types::evm::MulticallPluginConfig;

                let config = <$plugin_type as $crate::MulticallPlugin>::config();
                borsh::to_vec(&config).unwrap()
            }

            fn requests() -> Vec<u8> {
                use alloy::sol_types::{sol_data::Array, SolType};
                use evm_types::Call3;

                // Encode Vec<Call3> into ABI
                let calls = <$plugin_type as $crate::MulticallPlugin>::requests();
                <Array<Call3> as SolType>::abi_encode(&calls)
            }

            fn handle(state: Vec<u8>, results: Vec<u8>) -> Result<(Vec<u8>, Vec<u8>), String> {
                use alloy::sol_types::{sol_data::Array, SolType};
                use evm_types::{Call3, Result as MulticallResult};
                use rengine_types::Action;

                let state = if state.is_empty() {
                    <$plugin_type as $crate::MulticallPlugin>::State::default()
                } else {
                    borsh::from_slice(&state).unwrap_or_default()
                };

                // Decode Vec<MulticallResult> from ABI input
                let decoded: Vec<MulticallResult> =
                    <Array<MulticallResult> as SolType>::abi_decode(&results)
                        .map_err(|err| err.to_string())?;

                // Execute plugin logic
                let (new_state, actions) =
                    <$plugin_type as $crate::MulticallPlugin>::handle(state, decoded)?;

                let state_bytes = borsh::to_vec(&new_state).map_err(|err| err.to_string())?;
                let actions_bytes = borsh::to_vec(&actions).map_err(|err| err.to_string())?;
                Ok((state_bytes, actions_bytes))
            }
        }
    };
}

pub use crate::bindings::{export, Guest};
