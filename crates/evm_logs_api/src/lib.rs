pub mod bindings {
    use wit_bindgen::generate;
    generate!({path: "evm_logs.wit", pub_export_macro: true, export_macro_name: "export", });
}

pub use crate::bindings::{export, Guest};
use alloy::primitives::Log;
use evm_types::LogSubscription;
use rengine_types::Action;

pub trait EvmLogsPlugin {
    type State: borsh::BorshSerialize + borsh::BorshDeserialize + Default;

    fn init() -> (Self::State, LogSubscription);
    fn handle_log(state: Self::State, log: Log) -> (Self::State, Vec<Action>);
}

#[macro_export]
macro_rules! impl_guest_from_evm_logs_plugin {
    ($plugin_type:ty) => {
        impl $crate::bindings::Guest for $plugin_type {
            fn init() -> (Vec<u8>, Vec<u8>) {
                let (state, subscription) = <$plugin_type as $crate::EvmLogsPlugin>::init();
                (
                    borsh::to_vec(&state).unwrap(),
                    borsh::to_vec(&subscription).unwrap(),
                )
            }

            fn handle_log_message(state: Vec<u8>, log: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
                let state = if state.is_empty() {
                    <$plugin_type as $crate::EvmLogsPlugin>::State::default()
                } else {
                    borsh::from_slice(&state).unwrap_or_default()
                };

                let log: alloy::rpc::types::Log = serde_json::from_slice(&log).unwrap();
                let primitives_log = alloy::primitives::Log {
                    address: log.address(),
                    data: alloy::primitives::LogData::new_unchecked(
                        log.topics().to_vec(),
                        log.data().data.clone(),
                    ),
                };

                let (new_state, actions) =
                    <$plugin_type as $crate::EvmLogsPlugin>::handle_log(state, primitives_log);

                (
                    borsh::to_vec(&new_state).unwrap(),
                    borsh::to_vec(&actions).unwrap(),
                )
            }
        }
    };
}
