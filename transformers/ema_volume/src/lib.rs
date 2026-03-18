use anyhow::Result;
use rengine_types::{ExecutionRequest, StateUpdateKey, StrategyConfiguration, VenueBookKey};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::{collections::HashSet, time::Duration};
use strategy_api::{export, get_indicator, get_trade_flow, impl_guest_from_plugin, Plugin};

const INSTRUMENT: &str = "hyperliquid|eth/usdc-spot";

struct EmaVolume;

#[derive(Default, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct State;

impl Plugin for EmaVolume {
    type State = State;

    fn init() -> StrategyConfiguration {
        let mut keys = HashSet::new();
        // Run every 10 seconds
        keys.insert(StateUpdateKey::Timer {
            interval: Duration::from_secs(10),
        });
        StrategyConfiguration {
            triggers_keys: keys,
            cooldown: None,
        }
    }

    fn execute(state: Self::State) -> Result<(Self::State, Vec<ExecutionRequest>), String> {
        // Get trades since last execution
        let instrument_key: VenueBookKey = INSTRUMENT.parse().unwrap();
        let all_trades = get_trade_flow().unwrap_or_default();
        let trades = all_trades.get(&instrument_key).cloned().unwrap_or_default();

        let current_volume: Decimal = trades.iter().map(|t| t.size).sum();

        // Get previous EMA value (if exists)
        let prev_ema = get_indicator("eth_ema_volume").unwrap_or(current_volume);

        // Calculate new EMA
        // EMA = Volume(t) * k + EMA(y) * (1 – k)
        // k = 2 / (N + 1)
        // For N = 10, k = 2 / 11 ~= 0.1818
        let n = dec!(10);
        let k = dec!(2) / (n + dec!(1));

        let new_ema = current_volume * k + prev_ema * (dec!(1) - k);

        // Emit SetIndicator request
        Ok((
            state,
            vec![ExecutionRequest::SetIndicator(
                "eth_ema_volume".into(),
                new_ema,
            )],
        ))
    }
}

impl_guest_from_plugin!(EmaVolume, "ema_volume");

export!(EmaVolume with_types_in strategy_api::bindings);
