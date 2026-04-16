//! Shared type definitions for the `market_features` and `market_features_ema` transformers.
//!
//! This crate defines the state structs that both transformers operate on:
//! - [`MarketState`] — tick-level microstructure features, populated by `market_features`
//! - [`MarketEmaState`] — EMA-smoothed derivatives, populated by `market_features_ema`
//!
//! All structs are `#[repr(C)]` and implement [`Pod`](strategy_api::Pod) for zero-copy
//! WASM state management. The [`ToIndicators`] / [`FromIndicators`] derive macros
//! generate methods to emit/load indicator values with the `mf_` prefix.
//!
//! Indicator keys follow the pattern `mf_{market_id}_{field_name}`, where `market_id`
//! is produced by [`get_market_id`] (e.g., `mf_hyperliquid_eth_usd_perp_volume`).

use rengine_macros::{FromIndicators, ToIndicators};
use rust_decimal::Decimal;

/// Active instruments tracked by the market features transformers.
/// Format: `"venue|base/quote-market_type"`.
/// Uncomment entries as venues/pairs are onboarded.
pub const INSTRUMENTS: &[&str] = &[
    "hyperliquid|eth/usd-perp",
    // "hyperliquid|eth/usdc-spot",
    // "hyperliquid|btc/usdc-perp",
    // "hyperliquid|btc/usdc-spot",
    // "fbinance|btc/usdt-perp",
    // "fbinance|eth/usdt-perp",
    // "binance|btc/usdc-spot",
    // "binance|eth/usdc-spot",
];

/// Number of active instruments. Must match the length of [`INSTRUMENTS`].
pub const NUM_INSTRUMENTS: usize = 1;

/// Tick-level market microstructure state for a single instrument.
///
/// Populated by the `market_features` transformer on every trade-flow update.
/// All numeric fields are cumulative since the transformer started (not per-tick deltas),
/// except for `last_price`, `last_event_time`, and the `micro_cluster_*` working fields
/// which hold the current micro-cluster's in-progress values.
///
/// With the `c-repr` feature on `rust_decimal`, [`Decimal`] is `repr(C)` and safe for
/// [`Pod`](strategy_api::Pod) zero-copy operations.
#[repr(C)]
#[derive(Clone, Copy, Default, ToIndicators, FromIndicators)]
#[indicator(prefix = "mf_")]
pub struct MarketState {
    /// Cumulative traded volume in quote currency: Σ(size × price × contract_size).
    pub volume: Decimal,
    /// Cumulative log return: Σ ln(price_i / price_{i-1}).
    pub log_return: Decimal,
    /// Last observed trade price. Used as reference for the next log return calculation.
    pub last_price: Decimal,
    /// Timestamp (ms) of the last processed trade. Used for micro-cluster boundary detection.
    pub last_event_time: Decimal,

    // -- Micro-cluster metrics --
    // A "micro-cluster" is a group of trades sharing the same timestamp (microsecond batch).
    // These working fields accumulate within the current cluster and are finalized when
    // a new timestamp arrives.
    /// Cumulative count of finalized micro-clusters.
    pub micro_cluster_count: Decimal,
    /// Current micro-cluster's cumulative log return: Σ ln(p_i / p_{i-1}) within the cluster.
    pub micro_cluster_log: Decimal,
    /// Current micro-cluster's cumulative raw size: Σ size_i.
    pub micro_cluster_size: Decimal,
    /// Current micro-cluster's cumulative volume in quote: Σ(size × price).
    pub micro_cluster_volume: Decimal,
    /// Current micro-cluster's size-weighted log return: Σ(size_i × cumulative_log_i).
    /// Used for slippage/geometric mean calculations.
    pub micro_cluster_geom: Decimal,
    /// Current micro-cluster's size-weighted squared log return: Σ(size_i × cumulative_log_i²).
    pub micro_cluster_geom_sq: Decimal,

    // -- Slippage & liquidity --
    /// Cumulative absolute slippage: Σ |geom_ratio| where geom_ratio = geom / size per cluster.
    /// Measures how much price moves within each micro-cluster, size-weighted.
    pub slippage: Decimal,
    /// Cumulative squared slippage: Σ (geom_sq / size) per cluster.
    /// Second moment of within-cluster price impact.
    pub slippage_sq: Decimal,
    /// Cumulative adverse selection cost: Σ volume × (|log| - |geom_ratio|) per cluster.
    /// Positive means takers paid more than the size-weighted average — informed flow signal.
    pub antiselek: Decimal,
    /// Instantaneous micro-liquidity: volume / log² of the last finalized cluster.
    /// Higher = more volume needed to move price — deep liquidity.
    pub micro_liquidity: Decimal,

    // -- Micro-price tracking (3 timescales) --
    // Exponentially-weighted micro-price estimators at different decay rates,
    // using a parabolic market model (parameter α=2). "Hot" reacts fastest, "cold" slowest.
    /// Fast-decay micro-price: tracks recent price level with β = α/(1+α) ≈ 0.667.
    /// Equivalent to a short-window EWMA of log returns.
    pub micro_price_hot: Decimal,
    /// Medium-decay micro-price: β-weighted mean-reverting estimator.
    /// Increment = β × (log - micro_price_warm).
    pub micro_price_warm: Decimal,
    /// Slow-decay asymmetric micro-price: applies β for upward moves, (1-β) for downward.
    /// Captures directional bias in the price process.
    pub micro_price_cold: Decimal,

    /// Cumulative squared increment of hot micro-price. Tracks realized variance at the fast scale.
    pub micro_price_hot_variance: Decimal,
    /// Cumulative squared increment of warm micro-price.
    pub micro_price_warm_variance: Decimal,
    /// Cumulative squared increment of cold micro-price.
    pub micro_price_cold_variance: Decimal,

    // -- Mean reversion signals --
    /// Cumulative mean-reversion signal (hot): Σ micro_price_hot × cluster_log.
    /// Negative = price reverts after hot-scale moves.
    pub mean_reversion_hot: Decimal,
    /// Cumulative mean-reversion signal (warm): Σ micro_price_warm × cluster_log.
    pub mean_reversion_warm: Decimal,
    /// Cumulative mean-reversion signal (cold): Σ micro_price_cold × cluster_log.
    pub mean_reversion_cold: Decimal,
    /// Alternative mean-reversion metric using parabolic model:
    /// 0.5 × (α/(α+1)) × (w × increment_warm² - log²), where w = (α+2)/α.
    pub mean_reversion: Decimal,

    // -- PnL tracking per timescale --
    /// Cumulative PnL at hot scale: -Σ log × (log - micro_price_hot).
    /// Profit from mean-reversion at the fastest timescale.
    pub pnl_hot: Decimal,
    /// Cumulative PnL at warm scale: -Σ log × (log - micro_price_warm).
    pub pnl_warm: Decimal,
    /// Cumulative PnL at cold scale: -Σ log × (log - micro_price_cold).
    pub pnl_cold: Decimal,

    // -- Higher moments of per-cluster log returns --
    /// Cumulative absolute variation: Σ |cluster_log|. First moment of volatility.
    pub variation: Decimal,
    /// Cumulative variance: Σ cluster_log². Second moment (realized variance).
    pub variance: Decimal,
    /// Cumulative absolute skew: Σ |cluster_log³|. Third moment proxy.
    pub skew: Decimal,

    // -- Directional flow decomposition --
    /// Cumulative buy-side size: Σ cluster_size where taker is buyer (Side::Ask).
    pub size_up: Decimal,
    /// Cumulative sell-side size: Σ cluster_size where taker is seller (Side::Bid).
    pub size_dw: Decimal,
    /// Cumulative buy-side flow in quote: Σ cluster_volume for buyer-taker clusters.
    pub flow_up: Decimal,
    /// Cumulative sell-side flow in quote: Σ cluster_volume for seller-taker clusters.
    pub flow_dw: Decimal,
    /// Cumulative buy-side variance: Σ cluster_log² for buyer-taker clusters.
    pub var_up: Decimal,
    /// Cumulative sell-side variance: Σ cluster_log² for seller-taker clusters.
    pub var_dw: Decimal,
    /// Net signed trade flow: Σ ±cluster_volume (+ for buys, - for sells).
    pub trade_flow: Decimal,
    /// Directional variance asymmetry ("smile"): Σ ±cluster_log² (+ buys, - sells).
    /// Positive = more variance on the upside.
    pub smile: Decimal,

    // -- Retail flow detection --
    // Retail trades are identified by zero log return (price didn't move).
    /// Cumulative retail volume: Σ vol where log_return = 0.
    pub volume_retail: Decimal,
    /// Net signed retail flow: Σ ±vol where log_return = 0.
    pub trade_flow_retail: Decimal,
    /// Retail buy-side volume (seller-taker, price unchanged, retail absorbed on the bid).
    pub volume_retail_up: Decimal,
    /// Retail sell-side volume (buyer-taker, price unchanged, retail absorbed on the ask).
    pub volume_retail_down: Decimal,

    // -- Order book snapshot features (updated on top-of-book changes) --
    /// Log spread: ln(best_ask / best_bid). Tighter = more liquid.
    pub spread: Decimal,
    /// Spread-adjusted liquidity: (bid_notional + ask_notional) / (2 × spread²).
    /// Higher = deeper book relative to spread width.
    pub liquidity_imp: Decimal,
    /// Book imbalance: bid_notional - ask_notional. Positive = heavier bid side.
    pub liquidity_imbalance: Decimal,

    /// Variance-normalized liquidity: volume / variance.
    /// How much volume flows per unit of realized variance — efficiency metric.
    pub liquidity_real: Decimal,
    /// Directional liquidity (up): flow_up / var_up.
    /// Volume per unit variance on the buy side.
    pub liq_up: Decimal,
    /// Directional liquidity (down): flow_dw / var_dw.
    /// Volume per unit variance on the sell side.
    pub liq_dw: Decimal,
}

// SAFETY: MarketState is repr(C), Copy, contains only Decimal (which is Pod with c-repr)
unsafe impl strategy_api::Pod for MarketState {}

/// Top-level state for the `market_features` transformer.
/// Fixed-size array indexed by instrument position in [`INSTRUMENTS`].
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct State {
    pub markets: [MarketState; NUM_INSTRUMENTS],
}

// SAFETY: State is repr(C), Copy, contains only fixed-size arrays of Pod types
unsafe impl strategy_api::Pod for State {}

/// Returns the index of `instrument` in [`INSTRUMENTS`], or `None` if not found.
pub fn get_instrument_index(instrument: &str) -> Option<usize> {
    INSTRUMENTS.iter().position(|&i| i == instrument)
}

/// Converts an instrument string to a flat identifier for use in indicator keys.
///
/// Replaces `|`, `/`, and `-` with `_`.
/// Example: `"binance|btc/usdc-spot"` → `"binance_btc_usdc_spot"`.
pub fn get_market_id(instrument: &str) -> String {
    instrument.replace(|c| "|/-".contains(c), "_")
}

/// EMA-smoothed derivatives of [`MarketState`] for a single instrument.
///
/// Populated by the `market_features_ema` transformer every 60 seconds.
/// Fields fall into four categories:
/// - **`*_last`**: snapshot of the corresponding [`MarketState`] field from the previous tick,
///   used to compute deltas.
/// - **`*_delta`**: change since the last tick (`current - last`). Exposed as indicators for
///   downstream consumers.
/// - **`*_ema`**: exponential moving average of the delta (α = 0.01), updated as
///   `ema += α × (delta - ema)`.
/// - **Computed ratios**: derived from other EMA fields (e.g., `liquidity_real_ema = volume_ema / variance_ema`).
#[repr(C)]
#[derive(Clone, Copy, Default, ToIndicators)]
#[indicator(prefix = "mf_")]
pub struct MarketEmaState {
    // -- Last values (snapshots from previous tick for delta computation) --
    /// Previous tick's micro_cluster_count.
    pub micro_cluster_count_last: Decimal,
    /// Previous tick's cumulative log_return.
    pub log_return_last: Decimal,
    /// Previous tick's cumulative variation.
    pub variation_last: Decimal,
    /// Previous tick's cumulative variance.
    pub variance_last: Decimal,
    /// Previous tick's variance_felt (placeholder, not in MarketState).
    pub variance_felt_last: Decimal,
    /// Previous tick's cumulative skew.
    pub skew_last: Decimal,
    /// Previous tick's cumulative volume.
    pub volume_last: Decimal,
    /// Previous tick's cumulative retail volume.
    pub volume_retail_last: Decimal,
    /// Previous tick's cumulative retail buy volume.
    pub volume_retail_up_last: Decimal,
    /// Previous tick's cumulative retail sell volume.
    pub volume_retail_down_last: Decimal,
    /// Previous tick's cumulative antiselek.
    pub antiselek_last: Decimal,
    /// Previous tick's cumulative slippage.
    pub slippage_last: Decimal,
    /// Previous tick's cumulative slippage_sq.
    pub slippage_sq_last: Decimal,
    /// Previous tick's cumulative micro_price_hot_variance.
    pub micro_price_hot_variance_last: Decimal,
    /// Previous tick's cumulative micro_price_warm_variance.
    pub micro_price_warm_variance_last: Decimal,
    /// Previous tick's cumulative micro_price_cold_variance.
    pub micro_price_cold_variance_last: Decimal,
    /// Previous tick's cumulative mean_reversion.
    pub mean_reversion_last: Decimal,
    /// Previous tick's cumulative mean_reversion_hot.
    pub mean_reversion_hot_last: Decimal,
    /// Previous tick's cumulative mean_reversion_warm.
    pub mean_reversion_warm_last: Decimal,
    /// Previous tick's cumulative mean_reversion_cold.
    pub mean_reversion_cold_last: Decimal,
    /// Previous tick's cumulative pnl_hot.
    pub pnl_hot_last: Decimal,
    /// Previous tick's cumulative pnl_warm.
    pub pnl_warm_last: Decimal,
    /// Previous tick's cumulative pnl_cold.
    pub pnl_cold_last: Decimal,
    /// Previous tick's cumulative size_up.
    pub size_up_last: Decimal,
    /// Previous tick's cumulative size_dw.
    pub size_dw_last: Decimal,
    /// Previous tick's cumulative flow_up.
    pub flow_up_last: Decimal,
    /// Previous tick's cumulative flow_dw.
    pub flow_dw_last: Decimal,
    /// Previous tick's cumulative var_up.
    pub var_up_last: Decimal,
    /// Previous tick's cumulative var_dw.
    pub var_dw_last: Decimal,

    // -- Deltas (change since last tick) --
    /// Δ micro_cluster_count since last tick.
    pub micro_cluster_count_delta: Decimal,
    /// Δ log_return since last tick.
    pub log_return_delta: Decimal,
    /// Δ volume since last tick.
    pub volume_delta: Decimal,
    /// Δ variance since last tick.
    pub variance_delta: Decimal,
    /// Δ variation since last tick.
    pub variation_delta: Decimal,

    // -- EMAs (exponentially weighted moving averages of deltas, α = 0.01) --
    /// EMA of Δ micro_cluster_count. Activity rate.
    pub micro_cluster_count_ema: Decimal,
    /// EMA of Δ log_return. Smoothed return rate (drift).
    pub log_return_ema: Decimal,
    /// EMA of Δ variance. Smoothed realized variance rate.
    /// Initialized to 1e-10 to avoid division by zero.
    pub variance_ema: Decimal,
    /// EMA of variance_felt delta (placeholder).
    pub variance_felt_ema: Decimal,
    /// EMA of log_return_delta². Variance of returns (second moment of drift).
    pub variance2_ema: Decimal,
    /// EMA of Δ variation. Smoothed absolute return rate.
    pub variation_ema: Decimal,
    /// EMA of Δ skew. Smoothed skewness rate.
    pub skew_ema: Decimal,
    /// EMA of Δ volume. Smoothed volume rate.
    pub volume_ema: Decimal,
    /// EMA of Δ retail volume.
    pub volume_retail_ema: Decimal,
    /// EMA of Δ retail buy volume.
    pub volume_retail_up_ema: Decimal,
    /// EMA of Δ retail sell volume.
    pub volume_retail_down_ema: Decimal,
    /// EMA of trade_flow (instantaneous, not delta). Smoothed net signed flow.
    pub trade_flow_ema: Decimal,
    /// EMA of trade_flow_retail (instantaneous). Smoothed net retail flow.
    pub trade_flow_retail_ema: Decimal,
    /// EMA of Δ slippage. Smoothed price impact rate.
    pub slippage_ema: Decimal,
    /// EMA of Δ slippage_sq. Smoothed squared price impact rate.
    pub slippage_sq_ema: Decimal,
    /// EMA of Δ antiselek. Smoothed adverse selection rate.
    pub antiselek_ema: Decimal,
    /// EMA of Δ micro_price_hot_variance.
    pub micro_price_hot_variance_ema: Decimal,
    /// EMA of Δ micro_price_warm_variance.
    pub micro_price_warm_variance_ema: Decimal,
    /// EMA of Δ micro_price_cold_variance.
    pub micro_price_cold_variance_ema: Decimal,
    /// EMA of Δ mean_reversion.
    pub mean_reversion_ema: Decimal,
    /// EMA of Δ mean_reversion_hot.
    pub mean_reversion_hot_ema: Decimal,
    /// EMA of Δ mean_reversion_warm.
    pub mean_reversion_warm_ema: Decimal,
    /// EMA of Δ mean_reversion_cold.
    pub mean_reversion_cold_ema: Decimal,
    /// EMA of Δ pnl_hot.
    pub pnl_hot_ema: Decimal,
    /// EMA of Δ pnl_warm.
    pub pnl_warm_ema: Decimal,
    /// EMA of Δ pnl_cold.
    pub pnl_cold_ema: Decimal,
    /// EMA of Δ flow_up. Smoothed buy-side flow rate.
    pub flow_up_ema: Decimal,
    /// EMA of Δ flow_dw. Smoothed sell-side flow rate.
    pub flow_dw_ema: Decimal,
    /// EMA of Δ size_up. Smoothed buy-side size rate.
    pub size_up_ema: Decimal,
    /// EMA of Δ size_dw. Smoothed sell-side size rate.
    pub size_dw_ema: Decimal,
    /// EMA of Δ var_up. Smoothed buy-side variance rate.
    pub var_up_ema: Decimal,
    /// EMA of Δ var_dw. Smoothed sell-side variance rate.
    pub var_dw_ema: Decimal,
    /// EMA of liquidity_imp (instantaneous, not delta). Smoothed book-based liquidity.
    pub liquidity_imp_ema: Decimal,
    /// EMA of liquidity_imbalance (instantaneous). Smoothed bid-ask imbalance.
    pub liquidity_imbalance_ema: Decimal,
    /// Power metric: slip / (1 - slip) where slip = slippage_ema / variation_ema.
    /// Measures the ratio of within-cluster slippage to total price movement.
    /// Only updated when slip < 1 (bounded).
    pub power_ema: Decimal,
    /// Squared power metric: 2 × slip_sq / (1 - slip_sq) where slip_sq = slippage_sq_ema / variance_ema.
    /// Second-moment analogue of power_ema.
    pub power2_ema: Decimal,

    // -- Computed ratios (derived from other EMA fields, not directly smoothed) --
    /// Variance-normalized liquidity: volume_ema / variance_ema.
    /// How much volume flows per unit of variance — market efficiency proxy.
    pub liquidity_real_ema: Decimal,
    /// Buy-side liquidity: flow_up_ema / var_up_ema.
    pub liq_up_ema: Decimal,
    /// Sell-side liquidity: flow_dw_ema / var_dw_ema.
    pub liq_dw_ema: Decimal,
    /// Momentum: variance2_ema / variance_ema.
    /// Ratio of return-squared EMA to variance EMA. >1 means returns are clustered.
    pub momentum_ema: Decimal,
    /// Q-learning PnL estimate: 0.00005 × volume_ema - liquidity_real_ema × skew_ema / (power_ema + 1).
    pub q_pnl_ema: Decimal,
    /// Alternative Q-learning PnL: volume_ema × (0.00005 - variation_ema / (power_ema + 1)).
    pub q_pnl2_ema: Decimal,
    /// Q-learning performance: q_pnl_ema / volume_ema. PnL per unit volume.
    pub q_perf_ema: Decimal,
    /// Directional variance asymmetry: var_up_ema - var_dw_ema.
    /// Positive = more variance on buy side.
    pub smile_ema: Decimal,

    // -- Additional liquidity metrics --
    /// Squared liquidity_real_ema, for measuring liquidity variance/stability.
    pub liquidity_real_sq_ema: Decimal,
    /// Liquidity spread: liq_up_ema - liq_dw_ema. Directional liquidity imbalance.
    pub liq_spread_ema: Decimal,
    /// Liquidity ratio: liq_up_ema / liq_dw_ema. >1 means buy side is more liquid.
    /// Capped at 100 when denominator is zero, defaults to 1 when both are zero.
    pub liq_ratio_ema: Decimal,
    /// Total directional liquidity: liq_up_ema + liq_dw_ema.
    pub liq_total_ema: Decimal,
    /// Volume-variance ratio: volume_ema² / variance_ema.
    /// Higher = more stable liquidity regime.
    pub volume_variance_ratio_ema: Decimal,
}

// SAFETY: MarketEmaState is repr(C), Copy, contains only Decimal (which is Pod with c-repr)
unsafe impl strategy_api::Pod for MarketEmaState {}

/// Top-level state for the `market_features_ema` transformer.
/// Fixed-size array indexed by instrument position in [`INSTRUMENTS`].
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EmaState {
    pub markets: [MarketEmaState; NUM_INSTRUMENTS],
}

// SAFETY: EmaState is repr(C), Copy, contains only fixed-size arrays of Pod types
unsafe impl strategy_api::Pod for EmaState {}
