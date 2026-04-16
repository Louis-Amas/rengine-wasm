//! **Momentum & Regime Transformer** — multi-timeframe momentum, RSI, MACD, and volatility regime.
//!
//! Triggered on a 60-second [`Timer`](rengine_types::StateUpdateKey::Timer). Reads the
//! cumulative [`MarketState`](market_features_types::MarketState) indicators produced by
//! `market_features` and computes multi-scale derivatives for trend detection, overbought/
//! oversold signals, and volatility regime classification.
//!
//! **Inputs**: `mf_{market_id}_{field}` indicators (via `MarketState::from_indicators_with_market`).
//!
//! **Outputs**: `mr_{market_id}_{field}` indicators via [`ToIndicators`](rengine_macros::ToIndicators).
//!
//! ## Multi-scale architecture
//!
//! Three timescales are used throughout, with EMA smoothing factors:
//! - **Fast** (α = 0.1): ~10-sample effective window, reacts to recent moves.
//! - **Medium** (α = 0.01): ~100-sample window, captures intermediate trends.
//! - **Slow** (α = 0.001): ~1000-sample window, captures long-term drift.
//!
//! ## Key signals
//!
//! - **RSI** at three scales: 100 − 100/(1 + avg_gain/avg_loss). Range [0, 100].
//!   >70 overbought, <30 oversold. Multiple scales catch short-term vs structural extremes.
//! - **MACD**: return_fast − return_slow, with a signal line (EMA of MACD).
//!   Histogram (MACD − signal) shows momentum acceleration.
//! - **Vol term structure**: sqrt(vol_fast) / sqrt(vol_slow). >1 = short-term vol exceeds
//!   long-term ("inverted" → turbulent regime). <1 = calm trending market.
//! - **Trend strength**: |return_medium| / sqrt(vol_medium). ADX-like signal-to-noise ratio.
//!   Higher = stronger directional trend relative to noise.

use market_features_types::{get_market_id, MarketState, INSTRUMENTS, NUM_INSTRUMENTS};
use rengine_macros::ToIndicators;
use rengine_types::{ExecutionRequest, StateUpdateKey, StrategyConfiguration};
use rust_decimal::{Decimal, MathematicalOps};
use rust_decimal_macros::dec;
use std::{collections::HashSet, time::Duration};
use strategy_api::{export, impl_guest_from_unsafe_plugin, UnsafePlugin};

struct MomentumRegime;

/// Fast EMA smoothing factor (~10-sample effective window).
const ALPHA_FAST: Decimal = dec!(0.1);
/// Medium EMA smoothing factor (~100-sample effective window).
const ALPHA_MEDIUM: Decimal = dec!(0.01);
/// Slow EMA smoothing factor (~1000-sample effective window).
const ALPHA_SLOW: Decimal = dec!(0.001);

/// Per-instrument multi-timeframe momentum and volatility regime state.
///
/// Fields marked "(internal)" are intermediate state for computation but are still emitted
/// as indicators for observability and potential downstream use.
#[repr(C)]
#[derive(Clone, Copy, Default, ToIndicators)]
#[indicator(prefix = "mr_")]
pub struct MomentumState {
    // -- Multi-scale return EMAs --
    /// Fast EMA of log_return deltas (α = 0.1). Short-term drift.
    pub return_fast: Decimal,
    /// Medium EMA of log_return deltas (α = 0.01). Intermediate trend.
    pub return_medium: Decimal,
    /// Slow EMA of log_return deltas (α = 0.001). Long-term drift.
    pub return_slow: Decimal,

    // -- Multi-scale volume EMAs --
    /// Fast EMA of volume deltas (α = 0.1). Recent activity level.
    pub volume_fast: Decimal,
    /// Medium EMA of volume deltas (α = 0.01). Intermediate activity.
    pub volume_medium: Decimal,
    /// Slow EMA of volume deltas (α = 0.001). Baseline activity.
    pub volume_slow: Decimal,

    // -- RSI at three timescales --
    /// RSI (fast): 100 − 100/(1 + avg_gain_fast/avg_loss_fast).
    /// Range: [0, 100]. α = 0.1 ≈ RSI-14 equivalent. >70 overbought, <30 oversold.
    pub rsi_fast: Decimal,
    /// RSI (medium): α = 0.04 ≈ RSI-50 equivalent.
    pub rsi_medium: Decimal,
    /// RSI (slow): α = 0.01 ≈ RSI-200 equivalent. Structural overbought/oversold.
    pub rsi_slow: Decimal,

    /// (Internal) EMA of positive return deltas (fast scale). RSI numerator.
    pub avg_gain_fast: Decimal,
    /// (Internal) EMA of negative return deltas (fast scale). RSI denominator.
    pub avg_loss_fast: Decimal,
    /// (Internal) EMA of positive return deltas (medium scale).
    pub avg_gain_medium: Decimal,
    /// (Internal) EMA of negative return deltas (medium scale).
    pub avg_loss_medium: Decimal,
    /// (Internal) EMA of positive return deltas (slow scale).
    pub avg_gain_slow: Decimal,
    /// (Internal) EMA of negative return deltas (slow scale).
    pub avg_loss_slow: Decimal,

    // -- MACD --
    /// MACD line: return_fast − return_slow.
    /// Positive = short-term momentum exceeds long-term → bullish crossover.
    pub macd: Decimal,
    /// MACD signal line: EMA of MACD (α = 0.1).
    pub macd_signal: Decimal,
    /// MACD histogram: macd − macd_signal.
    /// Positive and growing = accelerating bullish momentum.
    pub macd_histogram: Decimal,

    // -- Realized volatility at three scales --
    /// Fast realized vol: EMA of return_delta² (α = 0.1). Recent volatility.
    pub vol_fast: Decimal,
    /// Medium realized vol: EMA of return_delta² (α = 0.01). Intermediate volatility.
    pub vol_medium: Decimal,
    /// Slow realized vol: EMA of return_delta² (α = 0.001). Baseline volatility.
    pub vol_slow: Decimal,

    // -- Volatility regime --
    /// Vol-of-vol: EMA of |Δvol_fast| (α = 0.01).
    /// Higher = volatility itself is volatile → regime transition.
    pub vol_of_vol: Decimal,
    /// (Internal) Previous vol_fast, for vol-of-vol delta computation.
    pub prev_vol_fast: Decimal,

    /// Vol term structure: sqrt(vol_fast) / sqrt(vol_slow).
    /// \>1 = inverted (short-term vol exceeds long-term, turbulent/event-driven).
    /// \<1 = normal contango (calm, trending market).
    /// Approx 1 = flat term structure (no strong regime signal).
    pub vol_term_structure: Decimal,

    // -- Trend strength --
    /// Trend strength (ADX-like): |return_medium| / sqrt(vol_medium).
    /// Signal-to-noise ratio of intermediate-term trend. Higher = stronger directional move.
    pub trend_strength: Decimal,

    // -- Delta tracking --
    /// (Internal) Previous mf.log_return value, for computing per-tick delta.
    pub prev_log_return: Decimal,
    /// (Internal) Previous mf.volume value, for computing per-tick delta.
    pub prev_volume: Decimal,
}

// SAFETY: MomentumState is repr(C), Copy, contains only Decimal (Pod with c-repr)
unsafe impl strategy_api::Pod for MomentumState {}

/// Top-level state: one [`MomentumState`] per instrument in [`INSTRUMENTS`].
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct State {
    pub markets: [MomentumState; NUM_INSTRUMENTS],
}

// SAFETY: State is repr(C), Copy, fixed-size array of Pod types
unsafe impl strategy_api::Pod for State {}

/// RSI smoothing factors for the three timescales.
/// These differ from the return/vol alphas to better match traditional RSI periods.
const RSI_ALPHA_FAST: Decimal = dec!(0.1); // ≈ RSI-14
const RSI_ALPHA_MEDIUM: Decimal = dec!(0.04); // ≈ RSI-50
const RSI_ALPHA_SLOW: Decimal = dec!(0.01); // ≈ RSI-200

impl UnsafePlugin for MomentumRegime {
    type State = State;

    fn init() -> StrategyConfiguration {
        let mut keys = HashSet::new();
        keys.insert(StateUpdateKey::Timer {
            interval: Duration::from_secs(60),
        });
        StrategyConfiguration {
            triggers_keys: keys,
            cooldown: None,
        }
    }

    fn execute(state: &mut Self::State) -> Result<Vec<ExecutionRequest>, String> {
        let mut requests = Vec::new();

        for (idx, instrument) in INSTRUMENTS.iter().enumerate() {
            let market_id = get_market_id(instrument);

            // Read current cumulative market features
            let mf = MarketState::from_indicators_with_market(&market_id);

            let momentum = &mut state.markets[idx];
            process_momentum(momentum, &mf);

            requests.extend(momentum.indicators_with_market(&market_id));
        }

        Ok(requests)
    }
}

/// Updates all momentum and volatility regime metrics for a single instrument.
///
/// Steps:
/// 1. Compute deltas from cumulative market_features values.
/// 2. Update multi-scale return and volume EMAs (fast/medium/slow).
/// 3. Compute RSI at three timescales from gain/loss EMAs.
/// 4. Update MACD (fast − slow), signal line, and histogram.
/// 5. Update realized volatility at three scales (EMA of return²).
/// 6. Compute vol-of-vol and vol term structure.
/// 7. Compute trend strength (signal-to-noise).
fn process_momentum(state: &mut MomentumState, mf: &MarketState) {
    // -- Compute deltas from cumulative values --
    let log_return_delta = mf.log_return - state.prev_log_return;
    let volume_delta = mf.volume - state.prev_volume;

    // -- Multi-scale return EMAs --
    state.return_fast += ALPHA_FAST * (log_return_delta - state.return_fast);
    state.return_medium += ALPHA_MEDIUM * (log_return_delta - state.return_medium);
    state.return_slow += ALPHA_SLOW * (log_return_delta - state.return_slow);

    // -- Multi-scale volume EMAs --
    state.volume_fast += ALPHA_FAST * (volume_delta - state.volume_fast);
    state.volume_medium += ALPHA_MEDIUM * (volume_delta - state.volume_medium);
    state.volume_slow += ALPHA_SLOW * (volume_delta - state.volume_slow);

    // -- RSI calculations --
    let gain = if log_return_delta > dec!(0) {
        log_return_delta
    } else {
        dec!(0)
    };
    let loss = if log_return_delta < dec!(0) {
        -log_return_delta
    } else {
        dec!(0)
    };

    // Fast RSI
    state.avg_gain_fast += RSI_ALPHA_FAST * (gain - state.avg_gain_fast);
    state.avg_loss_fast += RSI_ALPHA_FAST * (loss - state.avg_loss_fast);
    state.rsi_fast = compute_rsi(state.avg_gain_fast, state.avg_loss_fast);

    // Medium RSI
    state.avg_gain_medium += RSI_ALPHA_MEDIUM * (gain - state.avg_gain_medium);
    state.avg_loss_medium += RSI_ALPHA_MEDIUM * (loss - state.avg_loss_medium);
    state.rsi_medium = compute_rsi(state.avg_gain_medium, state.avg_loss_medium);

    // Slow RSI
    state.avg_gain_slow += RSI_ALPHA_SLOW * (gain - state.avg_gain_slow);
    state.avg_loss_slow += RSI_ALPHA_SLOW * (loss - state.avg_loss_slow);
    state.rsi_slow = compute_rsi(state.avg_gain_slow, state.avg_loss_slow);

    // -- MACD --
    state.macd = state.return_fast - state.return_slow;
    state.macd_signal += ALPHA_FAST * (state.macd - state.macd_signal);
    state.macd_histogram = state.macd - state.macd_signal;

    // -- Realized volatility (EMA of squared returns) --
    let return_sq = log_return_delta * log_return_delta;
    state.vol_fast += ALPHA_FAST * (return_sq - state.vol_fast);
    state.vol_medium += ALPHA_MEDIUM * (return_sq - state.vol_medium);
    state.vol_slow += ALPHA_SLOW * (return_sq - state.vol_slow);

    // -- Vol-of-vol --
    let vol_change = (state.vol_fast - state.prev_vol_fast).abs();
    state.vol_of_vol += ALPHA_MEDIUM * (vol_change - state.vol_of_vol);
    state.prev_vol_fast = state.vol_fast;

    // -- Vol term structure: sqrt(fast) / sqrt(slow) --
    let short_vol = state.vol_fast.sqrt().unwrap_or(dec!(0));
    let long_vol = state.vol_slow.sqrt().unwrap_or(dec!(0));
    state.vol_term_structure = if long_vol > dec!(0) {
        short_vol / long_vol
    } else {
        dec!(1)
    };

    // -- Trend strength: |return_medium| / sqrt(vol_medium) --
    state.trend_strength = if let Some(vol_med_sqrt) = state.vol_medium.sqrt() {
        if vol_med_sqrt > dec!(0) {
            state.return_medium.abs() / vol_med_sqrt
        } else {
            dec!(0)
        }
    } else {
        dec!(0)
    };

    // -- Update tracking --
    state.prev_log_return = mf.log_return;
    state.prev_volume = mf.volume;
}

/// Computes RSI from average gain and loss EMAs.
///
/// `RSI = 100 - 100 / (1 + avg_gain / avg_loss)`
///
/// Edge cases:
/// - avg_loss = 0, avg_gain > 0 → RSI = 100 (max overbought)
/// - Both zero → RSI = 50 (neutral)
fn compute_rsi(avg_gain: Decimal, avg_loss: Decimal) -> Decimal {
    if avg_loss > dec!(0) {
        dec!(100) - dec!(100) / (dec!(1) + avg_gain / avg_loss)
    } else if avg_gain > dec!(0) {
        dec!(100)
    } else {
        dec!(50)
    }
}

impl_guest_from_unsafe_plugin!(MomentumRegime, "momentum_regime");

export!(MomentumRegime with_types_in strategy_api::bindings);
