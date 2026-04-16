//! **Basis Features Transformer** — cross-instrument relationships for perpetual markets.
//!
//! Triggered on [`SetTopBook`](rengine_types::StateUpdateKey::SetTopBook) for all 8
//! instruments (the full set, independent of what `market_features` tracks). Computes
//! the carry signal (perp-spot basis), cross-venue spreads, cross-asset correlation,
//! and funding rate differentials.
//!
//! **Inputs**: `get_book(instrument)` for each instrument, `get_indicator(funding_key)`
//! for funding rates.
//!
//! **Outputs**: `bf_{pair_name}_{field}` indicators via [`ToIndicators`](rengine_macros::ToIndicators).
//!
//! ## Why this matters for perps
//!
//! The **basis** (ln(perp_price / spot_price)) is the defining signal of perpetual futures.
//! It reflects the cost of carry and funding pressure:
//! - Positive basis → perp trades at a premium → longs pay funding → crowded long.
//! - Negative basis → perp at a discount → shorts pay funding → crowded short.
//! - Basis z-score mean-reverts: extreme deviations tend to correct as funding normalizes.
//!
//! **Cross-venue spreads** detect arbitrage pressure and venue lead/lag: when one venue
//! moves first, the spread temporarily widens before the lagging venue catches up.
//!
//! **Cross-asset correlation** (BTC vs ETH) detects regime shifts: when correlation
//! breaks down, it often signals asset-specific events or structural dislocations.
//!
//! ## Graceful degradation
//!
//! When an instrument's book is unavailable (venue not connected, instrument not active),
//! `get_book()` returns an error and the corresponding pair/spread is skipped. The state
//! retains its previous values. This means the transformer produces useful output as
//! instruments come online — it doesn't require all 8 to function.
//!
//! ## Cooldown
//!
//! A 1-second cooldown prevents excessive recomputation since `SetTopBook` fires on
//! every best-bid/ask change across all 8 instruments.

use rengine_macros::ToIndicators;
use rengine_types::{ExecutionRequest, StateUpdateKey, StrategyConfiguration};
use rust_decimal::{Decimal, MathematicalOps};
use rust_decimal_macros::dec;
use std::{collections::HashSet, time::Duration};
use strategy_api::{export, get_book, get_indicator, impl_guest_from_unsafe_plugin, UnsafePlugin};

struct BasisFeatures;

// ---------------------------------------------------------------------------
// Instrument universe (full set, independent of market_features INSTRUMENTS)
// ---------------------------------------------------------------------------

/// All instruments needed for basis computation. Indices are used by [`PAIRS`] and
/// [`CROSS_VENUE`] to define relationships.
const ALL_INSTRUMENTS: &[&str] = &[
    "hyperliquid|eth/usd-perp",  // 0
    "hyperliquid|eth/usdc-spot", // 1
    "hyperliquid|btc/usd-perp",  // 2
    "hyperliquid|btc/usdc-spot", // 3
    "fbinance|btc/usdt-perp",    // 4
    "fbinance|eth/usdt-perp",    // 5
    "binance|btc/usdc-spot",     // 6
    "binance|eth/usdc-spot",     // 7
];

/// Number of perp-spot pairs (same venue, same underlying).
const NUM_PAIRS: usize = 4;
/// Number of cross-venue spread pairs.
const NUM_CROSS_VENUE: usize = 4;

/// Perp-spot pair definitions: (perp_instrument_idx, spot_instrument_idx, pair_name, funding_indicator_key).
const PAIRS: [(usize, usize, &str, &str); NUM_PAIRS] = [
    (0, 1, "eth_hl", "hyperliquid-funding-eth"),
    (2, 3, "btc_hl", "hyperliquid-funding-btc"),
    (5, 7, "eth_bn", "fbinance-funding-eth"),
    (4, 6, "btc_bn", "fbinance-funding-btc"),
];

/// Cross-venue spread definitions: (instrument_idx_a, instrument_idx_b, spread_name).
/// Spread = ln(mid_a / mid_b).
const CROSS_VENUE: [(usize, usize, &str); NUM_CROSS_VENUE] = [
    (0, 5, "eth_cross_perp"), // ETH: HL perp vs Binance perp
    (2, 4, "btc_cross_perp"), // BTC: HL perp vs Binance perp
    (1, 7, "eth_cross_spot"), // ETH: HL spot vs Binance spot
    (3, 6, "btc_cross_spot"), // BTC: HL spot vs Binance spot
];

// ---------------------------------------------------------------------------
// EMA parameters
// ---------------------------------------------------------------------------

/// EMA alpha for basis/spread smoothing and z-score variance.
const ALPHA_EMA: Decimal = dec!(0.01);
/// EMA alpha for basis velocity (faster to capture rate of change).
const ALPHA_VELOCITY: Decimal = dec!(0.05);
/// EMA alpha for cross-asset correlation tracking.
const ALPHA_CORR: Decimal = dec!(0.01);
/// Annualization factor: 365.25 × 24 × 3600 / (8 × 3600) = 1096.875.
/// Converts per-funding-interval basis to annualized rate (assuming 8h funding).
const ANNUALIZE: Decimal = dec!(1096.875);

// ---------------------------------------------------------------------------
// State structs
// ---------------------------------------------------------------------------

/// State for a single perp-spot basis pair (e.g., ETH on Hyperliquid).
///
/// Tracks the basis (carry signal), its EMA, variance, z-score, velocity, and funding rate.
/// Emitted with market prefix = pair name (e.g., `bf_eth_hl_basis`).
#[repr(C)]
#[derive(Clone, Copy, Default, ToIndicators)]
#[indicator(prefix = "bf_")]
pub struct PairState {
    /// Raw basis: ln(perp_mid / spot_mid).
    /// Positive = perp premium (longs pay funding).
    pub basis: Decimal,
    /// Annualized basis: basis × 1096.875 (from 8h funding interval).
    /// Expressed as annual rate (e.g., 0.15 = 15% annualized carry).
    pub basis_annualized: Decimal,

    /// EMA of basis (α = 0.01). Smoothed carry level.
    pub basis_ema: Decimal,
    /// EMA of (basis − basis_ema)² (α = 0.01). Basis variance for z-score.
    pub basis_var_ema: Decimal,
    /// Basis z-score: (basis − basis_ema) / sqrt(basis_var_ema).
    /// Mean-reverting signal. Typically distributed ~N(0,1).
    pub basis_zscore: Decimal,

    /// EMA of basis change (α = 0.05). Rate of basis movement.
    /// Positive = basis widening (funding pressure building).
    pub basis_velocity: Decimal,
    /// (Internal) Previous basis value, for velocity computation.
    pub prev_basis: Decimal,

    /// Current funding rate from the venue (read from indicator).
    pub funding_rate: Decimal,
}

// SAFETY: PairState is repr(C), Copy, all Decimal fields
unsafe impl strategy_api::Pod for PairState {}

/// State for a cross-venue spread (e.g., ETH HL perp vs Binance perp).
///
/// Tracks the inter-venue price differential, its EMA, and z-score.
/// Emitted with market prefix = spread name (e.g., `bf_eth_cross_perp_spread`).
#[repr(C)]
#[derive(Clone, Copy, Default, ToIndicators)]
#[indicator(prefix = "bf_")]
pub struct CrossVenueState {
    /// Cross-venue spread: ln(venue_a_mid / venue_b_mid).
    /// Positive = venue A trades at premium to venue B.
    pub spread: Decimal,
    /// EMA of spread (α = 0.01).
    pub spread_ema: Decimal,
    /// EMA of (spread − spread_ema)² (α = 0.01). Spread variance.
    pub spread_var_ema: Decimal,
    /// Spread z-score: (spread − spread_ema) / sqrt(spread_var_ema).
    /// Mean-reverting signal for cross-venue arbitrage.
    pub spread_zscore: Decimal,
}

// SAFETY: CrossVenueState is repr(C), Copy, all Decimal fields
unsafe impl strategy_api::Pod for CrossVenueState {}

/// State for cross-asset (BTC vs ETH) correlation and funding deltas.
///
/// Emitted with market prefix "cross" (e.g., `bf_cross_eth_btc_corr`).
#[repr(C)]
#[derive(Clone, Copy, Default, ToIndicators)]
#[indicator(prefix = "bf_")]
pub struct CrossAssetState {
    /// EMA-based Pearson correlation of ETH and BTC returns.
    /// Range: [-1, 1]. Computed from EMA of cross-product / sqrt(product of variances).
    /// Breakdown of correlation signals regime shifts or asset-specific events.
    pub eth_btc_corr: Decimal,
    /// (Internal) EMA of ETH return² (for correlation denominator).
    pub eth_return_var: Decimal,
    /// (Internal) EMA of BTC return² (for correlation denominator).
    pub btc_return_var: Decimal,
    /// (Internal) EMA of (ETH_return × BTC_return) (correlation numerator).
    pub cross_return_ema: Decimal,

    /// Funding rate delta for ETH: HL_funding − Binance_funding.
    /// Positive = ETH funding is higher on HL (carry advantage to short on HL).
    pub eth_funding_delta: Decimal,
    /// Funding rate delta for BTC: HL_funding − Binance_funding.
    pub btc_funding_delta: Decimal,

    /// (Internal) Previous ETH perp mid price, for return calculation.
    pub prev_eth_mid: Decimal,
    /// (Internal) Previous BTC perp mid price, for return calculation.
    pub prev_btc_mid: Decimal,
}

// SAFETY: CrossAssetState is repr(C), Copy, all Decimal fields
unsafe impl strategy_api::Pod for CrossAssetState {}

/// Top-level state for the basis_features transformer.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct State {
    /// One state per perp-spot pair. Indexed by [`PAIRS`] order.
    pub pairs: [PairState; NUM_PAIRS],
    /// One state per cross-venue spread. Indexed by [`CROSS_VENUE`] order.
    pub cross_venue: [CrossVenueState; NUM_CROSS_VENUE],
    /// Single cross-asset correlation and funding delta state.
    pub cross_asset: CrossAssetState,
}

// SAFETY: State is repr(C), Copy, all fields are Pod
unsafe impl strategy_api::Pod for State {}

// ---------------------------------------------------------------------------
// Plugin implementation
// ---------------------------------------------------------------------------

impl UnsafePlugin for BasisFeatures {
    type State = State;

    fn init() -> StrategyConfiguration {
        let mut keys = HashSet::new();
        for instrument in ALL_INSTRUMENTS {
            // Gracefully handle instruments that aren't parseable (shouldn't happen)
            if let Ok(key) = instrument.parse() {
                keys.insert(StateUpdateKey::SetTopBook(key));
            }
        }
        StrategyConfiguration {
            triggers_keys: keys,
            cooldown: Some(Duration::from_secs(1)),
        }
    }

    fn execute(state: &mut Self::State) -> Result<Vec<ExecutionRequest>, String> {
        // Read all mid prices. None = book unavailable for that instrument.
        let mids: Vec<Option<Decimal>> = ALL_INSTRUMENTS
            .iter()
            .map(|inst| get_book(inst).ok().map(|b| b.mid()))
            .collect();

        let mut requests = Vec::new();

        // -- Process perp-spot basis pairs --
        for (i, &(perp_idx, spot_idx, name, funding_key)) in PAIRS.iter().enumerate() {
            if let (Some(perp_mid), Some(spot_mid)) = (mids[perp_idx], mids[spot_idx]) {
                process_pair(&mut state.pairs[i], perp_mid, spot_mid, funding_key);
            }
            requests.extend(state.pairs[i].indicators_with_market(name));
        }

        // -- Process cross-venue spreads --
        for (i, &(idx_a, idx_b, name)) in CROSS_VENUE.iter().enumerate() {
            if let (Some(mid_a), Some(mid_b)) = (mids[idx_a], mids[idx_b]) {
                process_cross_venue(&mut state.cross_venue[i], mid_a, mid_b);
            }
            requests.extend(state.cross_venue[i].indicators_with_market(name));
        }

        // -- Process cross-asset correlation --
        // Use HL perp mids for ETH (idx 0) and BTC (idx 2)
        process_cross_asset(&mut state.cross_asset, mids[0], mids[2]);
        requests.extend(state.cross_asset.indicators_with_market("cross"));

        Ok(requests)
    }
}

// ---------------------------------------------------------------------------
// Processing functions
// ---------------------------------------------------------------------------

/// Updates basis metrics for a single perp-spot pair.
///
/// 1. Computes raw basis: `ln(perp_mid / spot_mid)`.
/// 2. Annualizes assuming 8h funding intervals.
/// 3. Updates basis EMA and variance EMA for z-score computation.
/// 4. Computes basis velocity (rate of change).
/// 5. Reads current funding rate from indicator system.
fn process_pair(state: &mut PairState, perp_mid: Decimal, spot_mid: Decimal, funding_key: &str) {
    if perp_mid <= dec!(0) || spot_mid <= dec!(0) {
        return;
    }

    // Raw basis
    state.basis = (perp_mid / spot_mid).checked_ln().unwrap_or(dec!(0));
    state.basis_annualized = state.basis * ANNUALIZE;

    // Basis EMA and z-score
    state.basis_ema += ALPHA_EMA * (state.basis - state.basis_ema);
    let deviation = state.basis - state.basis_ema;
    state.basis_var_ema += ALPHA_EMA * (deviation * deviation - state.basis_var_ema);
    state.basis_zscore = if state.basis_var_ema > dec!(0) {
        if let Some(std) = state.basis_var_ema.sqrt() {
            if std > dec!(0) {
                deviation / std
            } else {
                dec!(0)
            }
        } else {
            dec!(0)
        }
    } else {
        dec!(0)
    };

    // Basis velocity
    let basis_change = state.basis - state.prev_basis;
    state.basis_velocity += ALPHA_VELOCITY * (basis_change - state.basis_velocity);
    state.prev_basis = state.basis;

    // Funding rate (read-only, may return 0 if not yet populated)
    state.funding_rate = get_indicator(funding_key).unwrap_or(dec!(0));
}

/// Updates cross-venue spread metrics.
///
/// Computes `ln(mid_a / mid_b)`, updates EMA, variance, and z-score.
fn process_cross_venue(state: &mut CrossVenueState, mid_a: Decimal, mid_b: Decimal) {
    if mid_a <= dec!(0) || mid_b <= dec!(0) {
        return;
    }

    state.spread = (mid_a / mid_b).checked_ln().unwrap_or(dec!(0));

    state.spread_ema += ALPHA_EMA * (state.spread - state.spread_ema);
    let deviation = state.spread - state.spread_ema;
    state.spread_var_ema += ALPHA_EMA * (deviation * deviation - state.spread_var_ema);
    state.spread_zscore = if state.spread_var_ema > dec!(0) {
        if let Some(std) = state.spread_var_ema.sqrt() {
            if std > dec!(0) {
                deviation / std
            } else {
                dec!(0)
            }
        } else {
            dec!(0)
        }
    } else {
        dec!(0)
    };
}

/// Updates cross-asset (BTC vs ETH) correlation and funding deltas.
///
/// Uses EMA-based Pearson correlation:
/// `corr = ema(ret_eth × ret_btc) / sqrt(ema(ret_eth²) × ema(ret_btc²))`
fn process_cross_asset(
    state: &mut CrossAssetState,
    eth_mid: Option<Decimal>,
    btc_mid: Option<Decimal>,
) {
    // Compute returns if we have both current and previous mids
    if let (Some(eth_mid), Some(btc_mid)) = (eth_mid, btc_mid) {
        if state.prev_eth_mid > dec!(0)
            && state.prev_btc_mid > dec!(0)
            && eth_mid > dec!(0)
            && btc_mid > dec!(0)
        {
            let eth_ret = (eth_mid / state.prev_eth_mid)
                .checked_ln()
                .unwrap_or(dec!(0));
            let btc_ret = (btc_mid / state.prev_btc_mid)
                .checked_ln()
                .unwrap_or(dec!(0));

            // Update variance and covariance EMAs
            state.eth_return_var += ALPHA_CORR * (eth_ret * eth_ret - state.eth_return_var);
            state.btc_return_var += ALPHA_CORR * (btc_ret * btc_ret - state.btc_return_var);
            state.cross_return_ema += ALPHA_CORR * (eth_ret * btc_ret - state.cross_return_ema);

            // Pearson correlation
            let denom_sq = state.eth_return_var * state.btc_return_var;
            state.eth_btc_corr = if denom_sq > dec!(0) {
                if let Some(denom) = denom_sq.sqrt() {
                    if denom > dec!(0) {
                        state.cross_return_ema / denom
                    } else {
                        dec!(0)
                    }
                } else {
                    dec!(0)
                }
            } else {
                dec!(0)
            };
        }

        state.prev_eth_mid = eth_mid;
        state.prev_btc_mid = btc_mid;
    }

    // Funding rate deltas (HL - Binance)
    let eth_hl_funding = get_indicator("hyperliquid-funding-eth").unwrap_or(dec!(0));
    let eth_bn_funding = get_indicator("fbinance-funding-eth").unwrap_or(dec!(0));
    state.eth_funding_delta = eth_hl_funding - eth_bn_funding;

    let btc_hl_funding = get_indicator("hyperliquid-funding-btc").unwrap_or(dec!(0));
    let btc_bn_funding = get_indicator("fbinance-funding-btc").unwrap_or(dec!(0));
    state.btc_funding_delta = btc_hl_funding - btc_bn_funding;
}

impl_guest_from_unsafe_plugin!(BasisFeatures, "basis_features");

export!(BasisFeatures with_types_in strategy_api::bindings);
