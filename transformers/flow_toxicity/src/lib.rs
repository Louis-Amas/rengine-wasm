//! **Flow Toxicity Transformer** — informed flow detection and adverse selection signals.
//!
//! Triggered on [`SetTradeFlow`](rengine_types::StateUpdateKey::SetTradeFlow) for each
//! active instrument. Processes raw trade batches to detect signatures of informed trading:
//! order flow imbalance (VPIN), price impact (Kyle's lambda), flow persistence, and
//! institutional vs. retail trade size patterns.
//!
//! **Inputs**: `get_trade_flow()` — all recent public trades grouped by instrument.
//!
//! **Outputs**: `ft_{market_id}_{field}` indicators via [`ToIndicators`](rengine_macros::ToIndicators).
//!
//! ## Key signals
//!
//! - **VPIN**: Volume-synchronized Probability of Informed Trading. EMA approximation of
//!   the absolute order imbalance normalized by total volume. Range [0, 1]. Higher values
//!   indicate more one-sided (potentially informed) flow.
//! - **Kyle's lambda (λ)**: Price impact coefficient — how much price moves per unit of
//!   signed flow. Higher λ means the market is thinner or flow is more toxic.
//! - **Flow autocorrelation**: Persistence of signed flow direction. Positive = trending
//!   (informed traders splitting orders), negative = mean-reverting (noise).
//! - **Large trade ratio**: Fraction of volume from trades exceeding 2× the average size.
//!   Institutional flow tends to cluster in larger sizes.
//! - **Signed flow momentum**: Fast and slow EMAs of net signed flow, enabling crossover
//!   signals (fast > slow = buying pressure building).

use market_features_types::{get_market_id, INSTRUMENTS, NUM_INSTRUMENTS};
use rengine_macros::ToIndicators;
use rengine_types::{
    ExecutionRequest, PublicTrade, Side, StateUpdateKey, StrategyConfiguration, VenueBookKey,
};
use rust_decimal::{Decimal, MathematicalOps};
use rust_decimal_macros::dec;
use std::collections::HashSet;
use strategy_api::{export, get_trade_flow, impl_guest_from_unsafe_plugin, UnsafePlugin};

struct FlowToxicity;

/// Primary EMA smoothing factor for most toxicity metrics.
const ALPHA: Decimal = dec!(0.05);
/// Fast EMA smoothing factor for signed flow momentum.
const ALPHA_FAST: Decimal = dec!(0.1);
/// Slow EMA smoothing factor for signed flow momentum.
const ALPHA_SLOW: Decimal = dec!(0.01);

/// Per-instrument flow toxicity state.
///
/// All fields are [`Decimal`] for Pod compatibility. Fields marked "(internal)" are
/// intermediate state needed for computation but are still emitted as indicators for
/// observability.
#[repr(C)]
#[derive(Clone, Copy, Default, ToIndicators)]
#[indicator(prefix = "ft_")]
pub struct FlowState {
    /// VPIN (Volume-synchronized Probability of Informed Trading).
    /// EMA of |buy_vol - sell_vol| / total_vol (α = 0.05).
    /// Range: [0, 1]. Higher = more toxic/informed flow.
    pub vpin: Decimal,

    /// Kyle's lambda (λ) — price impact coefficient.
    /// EMA of |Δln(price)| / |signed_flow| (α = 0.05).
    /// Units: log-price-change per unit of quote flow. Small positive value.
    pub kyle_lambda: Decimal,

    /// Flow autocorrelation: flow_cross_ema / flow_sq_ema.
    /// Range: [-1, 1]. Positive = persistent/trending flow, negative = mean-reverting.
    pub flow_autocorr: Decimal,
    /// (Internal) EMA of signed_flow_t × prev_signed_flow.
    pub flow_cross_ema: Decimal,
    /// (Internal) EMA of signed_flow².
    pub flow_sq_ema: Decimal,

    /// Large trade ratio: EMA of large_trade_volume / total_volume.
    /// "Large" = individual trade size > 2× avg_trade_size_ema.
    /// Higher = more institutional/whale activity.
    pub large_trade_ratio: Decimal,
    /// (Internal) EMA of average trade size (quote volume per trade).
    pub avg_trade_size_ema: Decimal,

    /// Trade intensity: EMA of trade count per batch.
    /// Higher = more active market.
    pub trade_intensity: Decimal,

    /// Fast signed flow momentum: EMA of net signed flow (α = 0.1).
    /// Positive = recent buying pressure, negative = selling pressure.
    pub signed_flow_fast: Decimal,
    /// Slow signed flow momentum: EMA of net signed flow (α = 0.01).
    /// Crossover (fast > slow) signals building directional pressure.
    pub signed_flow_slow: Decimal,

    /// Flow-volatility correlation: abs_flow_ret_ema / sqrt(flow_sq_ema × ret_sq_ema).
    /// Range: [0, 1]. Higher = flow and volatility are coupled (informed trading).
    pub flow_vol_corr: Decimal,
    /// (Internal) EMA of |signed_flow| × |price_return|.
    pub abs_flow_ret_ema: Decimal,
    /// (Internal) EMA of price_return².
    pub ret_sq_ema: Decimal,

    /// (Internal) Previous batch's signed flow, for autocorrelation computation.
    pub prev_flow: Decimal,
    /// (Internal) Last observed trade price, for return computation.
    pub prev_price: Decimal,
}

// SAFETY: FlowState is repr(C), Copy, contains only Decimal (Pod with c-repr)
unsafe impl strategy_api::Pod for FlowState {}

/// Top-level state: one [`FlowState`] per instrument in [`INSTRUMENTS`].
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct State {
    pub markets: [FlowState; NUM_INSTRUMENTS],
}

// SAFETY: State is repr(C), Copy, fixed-size array of Pod types
unsafe impl strategy_api::Pod for State {}

impl UnsafePlugin for FlowToxicity {
    type State = State;

    fn init() -> StrategyConfiguration {
        let mut keys = HashSet::new();
        for instrument in INSTRUMENTS {
            keys.insert(StateUpdateKey::SetTradeFlow(instrument.parse().unwrap()));
        }
        StrategyConfiguration {
            triggers_keys: keys,
            cooldown: None,
        }
    }

    fn execute(state: &mut Self::State) -> Result<Vec<ExecutionRequest>, String> {
        let all_trades = get_trade_flow().unwrap_or_default();
        let mut requests = Vec::new();

        for (idx, instrument) in INSTRUMENTS.iter().enumerate() {
            let instrument_key: VenueBookKey = instrument.parse().unwrap();
            let market_id = get_market_id(instrument);
            let flow_state = &mut state.markets[idx];

            if let Some(trades) = all_trades.get(&instrument_key) {
                if !trades.is_empty() {
                    process_flow(flow_state, trades);
                }
            }

            requests.extend(flow_state.indicators_with_market(&market_id));
        }

        Ok(requests)
    }
}

/// Processes a batch of trades and updates all flow toxicity metrics.
///
/// Steps:
/// 1. Aggregate batch: split volume into buy/sell sides, detect large trades,
///    record the last trade price.
/// 2. Compute VPIN: `ema(|buy_vol - sell_vol| / total_vol)`.
/// 3. Compute Kyle's λ: `ema(|log_return| / |signed_flow|)`.
/// 4. Update flow autocorrelation: `ema(flow_t × prev_flow) / ema(flow²)`.
/// 5. Update large trade ratio: `ema(large_vol / total_vol)`.
/// 6. Update trade intensity and signed flow momentum at two speeds.
/// 7. Compute flow-volatility correlation.
fn process_flow(state: &mut FlowState, trades: &[PublicTrade]) {
    if trades.is_empty() {
        return;
    }

    // -- Aggregate batch statistics --
    let mut buy_vol = dec!(0);
    let mut sell_vol = dec!(0);
    let mut total_vol = dec!(0);
    let mut large_trade_vol = dec!(0);
    let mut last_price = dec!(0);
    let trade_count = Decimal::from(trades.len() as u64);

    for trade in trades {
        let vol = trade.size * trade.price;
        total_vol += vol;

        match trade.side {
            Side::Ask => buy_vol += vol,  // buyer is taker
            Side::Bid => sell_vol += vol, // seller is taker
        }

        // Large trade detection: individual trade volume > 2× rolling average
        if state.avg_trade_size_ema > dec!(0) && vol > dec!(2) * state.avg_trade_size_ema {
            large_trade_vol += vol;
        }

        last_price = trade.price;
    }

    if total_vol == dec!(0) {
        return;
    }

    let signed_flow = buy_vol - sell_vol;
    let avg_trade_size = total_vol / trade_count;

    // -- Update average trade size EMA --
    state.avg_trade_size_ema += ALPHA * (avg_trade_size - state.avg_trade_size_ema);

    // -- VPIN: order flow imbalance --
    let imbalance = (buy_vol - sell_vol).abs() / total_vol;
    state.vpin += ALPHA * (imbalance - state.vpin);

    // -- Kyle's lambda: price impact per unit flow --
    let price_return = if state.prev_price > dec!(0) && last_price > dec!(0) {
        (last_price / state.prev_price)
            .checked_ln()
            .unwrap_or(dec!(0))
    } else {
        dec!(0)
    };

    let signed_flow_abs = signed_flow.abs();
    if signed_flow_abs > dec!(0) {
        let lambda_sample = price_return.abs() / signed_flow_abs;
        state.kyle_lambda += ALPHA * (lambda_sample - state.kyle_lambda);
    }

    // -- Flow autocorrelation --
    let cross = signed_flow * state.prev_flow;
    state.flow_cross_ema += ALPHA * (cross - state.flow_cross_ema);
    state.flow_sq_ema += ALPHA * (signed_flow * signed_flow - state.flow_sq_ema);
    state.flow_autocorr = if state.flow_sq_ema > dec!(0) {
        state.flow_cross_ema / state.flow_sq_ema
    } else {
        dec!(0)
    };

    // -- Large trade ratio --
    let large_ratio = large_trade_vol / total_vol;
    state.large_trade_ratio += ALPHA * (large_ratio - state.large_trade_ratio);

    // -- Trade intensity --
    state.trade_intensity += ALPHA * (trade_count - state.trade_intensity);

    // -- Signed flow momentum (two speeds) --
    state.signed_flow_fast += ALPHA_FAST * (signed_flow - state.signed_flow_fast);
    state.signed_flow_slow += ALPHA_SLOW * (signed_flow - state.signed_flow_slow);

    // -- Flow-volatility correlation --
    let abs_flow_ret = signed_flow_abs * price_return.abs();
    state.abs_flow_ret_ema += ALPHA * (abs_flow_ret - state.abs_flow_ret_ema);
    state.ret_sq_ema += ALPHA * (price_return * price_return - state.ret_sq_ema);

    let denom_sq = state.flow_sq_ema * state.ret_sq_ema;
    state.flow_vol_corr = if denom_sq > dec!(0) {
        if let Some(denom) = denom_sq.sqrt() {
            state.abs_flow_ret_ema / denom
        } else {
            dec!(0)
        }
    } else {
        dec!(0)
    };

    // -- Update tracking state --
    state.prev_flow = signed_flow;
    state.prev_price = last_price;
}

impl_guest_from_unsafe_plugin!(FlowToxicity, "flow_toxicity");

export!(FlowToxicity with_types_in strategy_api::bindings);
