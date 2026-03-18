use alloy::{
    primitives::{address, Address, Log},
    sol,
    sol_types::SolEvent,
};
use borsh::{BorshDeserialize, BorshSerialize};
use evm_logs_api::{impl_guest_from_evm_logs_plugin, EvmLogsPlugin};
use evm_types::{u256_to_decimal_with_scale, LogSubscription};
use rengine_types::Action;
use rust_decimal::Decimal;
use std::time::Duration;

sol! {
    event Transfer(address indexed from, address indexed to, uint256 value);
}

const MOCK_ERC20: Address = address!("0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512");

#[derive(BorshSerialize, BorshDeserialize)]
struct State {
    cumulative_volume: Decimal,
    decimals: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            cumulative_volume: Decimal::ZERO,
            decimals: 18,
        }
    }
}

struct Component;

impl EvmLogsPlugin for Component {
    type State = State;

    fn init() -> (Self::State, LogSubscription) {
        let subscription = LogSubscription {
            address: MOCK_ERC20.into(),
            topics: vec![Transfer::SIGNATURE_HASH.into()],
            timeout: Some(Duration::from_secs(600)), // 10 minutes
        };
        (State::default(), subscription)
    }

    fn handle_log(mut state: Self::State, log: Log) -> (Self::State, Vec<Action>) {
        // Check if it matches our event
        if log.data.topics().first() != Some(&Transfer::SIGNATURE_HASH) {
            return (state, vec![]);
        }

        if let Ok(transfer) = Transfer::decode_log(&log) {
            // The evm_integration_test strategy sends a transfer to 0x0202...
            // We only want to react to that specific transfer to verify integration.
            let target_recipient = Address::from([0x02; 20]);
            if transfer.to == target_recipient {
                let decimal = u256_to_decimal_with_scale(transfer.data.value, state.decimals);
                state.cumulative_volume = state
                    .cumulative_volume
                    .checked_add(decimal)
                    .unwrap_or(state.cumulative_volume);

                let actions = vec![Action::SetIndicator(
                    "cumulative_transfer_volume".into(),
                    state.cumulative_volume,
                )];
                return (state, actions);
            }
        }

        (state, vec![])
    }
}

impl_guest_from_evm_logs_plugin!(Component);

evm_logs_api::bindings::export!(Component with_types_in evm_logs_api::bindings);
