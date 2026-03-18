use anyhow::Result;
use rengine_types::{ExecutionRequest, StateUpdateKey, StrategyConfiguration};
use rust_decimal_macros::dec;
use std::{collections::HashSet, time::Duration};
use strategy_api::{export, get_book, get_indicator, impl_guest_from_plugin, Plugin};

struct EmaPrice;

#[derive(Default, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct State;

impl Plugin for EmaPrice {
    type State = State;

    fn init() -> StrategyConfiguration {
        let mut keys = HashSet::new();
        // Run every minute
        keys.insert(StateUpdateKey::UtcTimer {
            interval: Duration::from_secs(60),
        });
        StrategyConfiguration {
            triggers_keys: keys,
            cooldown: None,
        }
    }

    fn execute(state: Self::State) -> Result<(Self::State, Vec<ExecutionRequest>), String> {
        // Get current top book price
        let book = get_book("hyperliquid|eth/usdc-spot").map_err(|e| e.to_string())?;
        let mid_price = book.mid();

        // Get previous EMA value (if exists)
        let prev_ema = get_indicator("eth_ema_price").unwrap_or(mid_price); // Initialize with current price if no previous EMA

        // Calculate new EMA
        // EMA = Price(t) * k + EMA(y) * (1 – k)
        // k = 2 / (N + 1)
        // For N = 10, k = 2 / 11 ~= 0.1818
        let n = dec!(10);
        let k = dec!(2) / (n + dec!(1));

        let new_ema = mid_price * k + prev_ema * (dec!(1) - k);

        // Emit SetIndicator request
        Ok((
            state,
            vec![ExecutionRequest::SetIndicator(
                "eth_ema_price".into(),
                new_ema,
            )],
        ))
    }
}

impl_guest_from_plugin!(EmaPrice, "ema_price");

export!(EmaPrice with_types_in strategy_api::bindings);
