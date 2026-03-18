use rengine_macros::{FromIndicators, ToIndicators};
use rust_decimal::Decimal;

pub const INSTRUMENTS: &[&str] = &[
    "hyperliquid|eth/usdc-perp",
    "hyperliquid|eth/usdc-spot",
    "hyperliquid|btc/usdc-perp",
    "hyperliquid|btc/usdc-spot",
    "fbinance|btc/usdt-perp",
    "fbinance|eth/usdt-perp",
    "binance|btc/usdc-spot",
    "binance|eth/usdc-spot",
];

pub const NUM_INSTRUMENTS: usize = 8;

/// MarketState for tracking market features.
/// With the "c-repr" feature on rust_decimal, Decimal is repr(C) and safe for Pod operations.
#[repr(C)]
#[derive(Clone, Copy, Default, ToIndicators, FromIndicators)]
#[indicator(prefix = "mf_")]
pub struct MarketState {
    pub volume: Decimal,
    pub log_return: Decimal,
    pub last_price: Decimal,
    pub last_event_time: Decimal,

    pub micro_cluster_count: Decimal,
    pub micro_cluster_log: Decimal,
    pub micro_cluster_size: Decimal,
    pub micro_cluster_volume: Decimal,
    pub micro_cluster_geom: Decimal,
    pub micro_cluster_geom_sq: Decimal,

    pub slippage: Decimal,
    pub slippage_sq: Decimal,
    pub antiselek: Decimal,
    pub micro_liquidity: Decimal,

    pub micro_price_hot: Decimal,
    pub micro_price_warm: Decimal,
    pub micro_price_cold: Decimal,

    pub micro_price_hot_variance: Decimal,
    pub micro_price_warm_variance: Decimal,
    pub micro_price_cold_variance: Decimal,

    pub mean_reversion_hot: Decimal,
    pub mean_reversion_warm: Decimal,
    pub mean_reversion_cold: Decimal,
    pub mean_reversion: Decimal,

    pub pnl_hot: Decimal,
    pub pnl_warm: Decimal,
    pub pnl_cold: Decimal,

    pub variation: Decimal,
    pub variance: Decimal,
    pub skew: Decimal,

    pub size_up: Decimal,
    pub size_dw: Decimal,
    pub flow_up: Decimal,
    pub flow_dw: Decimal,
    pub var_up: Decimal,
    pub var_dw: Decimal,
    pub trade_flow: Decimal,
    pub smile: Decimal,

    pub volume_retail: Decimal,
    pub trade_flow_retail: Decimal,
    pub volume_retail_up: Decimal,
    pub volume_retail_down: Decimal,

    pub spread: Decimal,
    pub liquidity_imp: Decimal,
    pub liquidity_imbalance: Decimal,

    // Liquidity based on variance and volumes
    pub liquidity_real: Decimal,
    pub liq_up: Decimal,
    pub liq_dw: Decimal,
}

// SAFETY: MarketState is repr(C), Copy, contains only Decimal (which is Pod with c-repr)
unsafe impl strategy_api::Pod for MarketState {}

/// State for market_features - fixed-size array of markets
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct State {
    pub markets: [MarketState; NUM_INSTRUMENTS],
}

// SAFETY: State is repr(C), Copy, contains only fixed-size arrays of Pod types
unsafe impl strategy_api::Pod for State {}

/// Get the index of an instrument in the INSTRUMENTS array
pub fn get_instrument_index(instrument: &str) -> Option<usize> {
    INSTRUMENTS.iter().position(|&i| i == instrument)
}

/// Extracts a market identifier from the full instrument string
/// e.g., "binance|btc/usdc-spot" -> "binance_btc_usdc_spot"
pub fn get_market_id(instrument: &str) -> String {
    instrument.replace(|c| "|/-".contains(c), "_")
}

/// MarketEmaState for tracking EMA calculations.
/// With the "c-repr" feature on rust_decimal, Decimal is repr(C) and safe for Pod operations.
#[repr(C)]
#[derive(Clone, Copy, Default, ToIndicators)]
#[indicator(prefix = "mf_")]
pub struct MarketEmaState {
    // Last values
    pub micro_cluster_count_last: Decimal,
    pub log_return_last: Decimal,
    pub variation_last: Decimal,
    pub variance_last: Decimal,
    pub variance_felt_last: Decimal,
    pub skew_last: Decimal,
    pub volume_last: Decimal,
    pub volume_retail_last: Decimal,
    pub volume_retail_up_last: Decimal,
    pub volume_retail_down_last: Decimal,
    pub antiselek_last: Decimal,
    pub slippage_last: Decimal,
    pub slippage_sq_last: Decimal,
    pub micro_price_hot_variance_last: Decimal,
    pub micro_price_warm_variance_last: Decimal,
    pub micro_price_cold_variance_last: Decimal,
    pub mean_reversion_last: Decimal,
    pub mean_reversion_hot_last: Decimal,
    pub mean_reversion_warm_last: Decimal,
    pub mean_reversion_cold_last: Decimal,
    pub pnl_hot_last: Decimal,
    pub pnl_warm_last: Decimal,
    pub pnl_cold_last: Decimal,
    pub size_up_last: Decimal,
    pub size_dw_last: Decimal,
    pub flow_up_last: Decimal,
    pub flow_dw_last: Decimal,
    pub var_up_last: Decimal,
    pub var_dw_last: Decimal,

    // Deltas
    pub micro_cluster_count_delta: Decimal,
    pub log_return_delta: Decimal,
    pub volume_delta: Decimal,
    pub variance_delta: Decimal,
    pub variation_delta: Decimal,

    // EMAs
    pub micro_cluster_count_ema: Decimal,
    pub log_return_ema: Decimal,
    pub variance_ema: Decimal,
    pub variance_felt_ema: Decimal,
    pub variance2_ema: Decimal,
    pub variation_ema: Decimal,
    pub skew_ema: Decimal,
    pub volume_ema: Decimal,
    pub volume_retail_ema: Decimal,
    pub volume_retail_up_ema: Decimal,
    pub volume_retail_down_ema: Decimal,
    pub trade_flow_ema: Decimal,
    pub trade_flow_retail_ema: Decimal,
    pub slippage_ema: Decimal,
    pub slippage_sq_ema: Decimal,
    pub antiselek_ema: Decimal,
    pub micro_price_hot_variance_ema: Decimal,
    pub micro_price_warm_variance_ema: Decimal,
    pub micro_price_cold_variance_ema: Decimal,
    pub mean_reversion_ema: Decimal,
    pub mean_reversion_hot_ema: Decimal,
    pub mean_reversion_warm_ema: Decimal,
    pub mean_reversion_cold_ema: Decimal,
    pub pnl_hot_ema: Decimal,
    pub pnl_warm_ema: Decimal,
    pub pnl_cold_ema: Decimal,
    pub flow_up_ema: Decimal,
    pub flow_dw_ema: Decimal,
    pub size_up_ema: Decimal,
    pub size_dw_ema: Decimal,
    pub var_up_ema: Decimal,
    pub var_dw_ema: Decimal,
    pub liquidity_imp_ema: Decimal,
    pub liquidity_imbalance_ema: Decimal,
    pub power_ema: Decimal,
    pub power2_ema: Decimal,

    // Computed EMAs / Ratios
    pub liquidity_real_ema: Decimal,
    pub liq_up_ema: Decimal,
    pub liq_dw_ema: Decimal,
    pub momentum_ema: Decimal,
    pub q_pnl_ema: Decimal,
    pub q_pnl2_ema: Decimal,
    pub q_perf_ema: Decimal,
    pub smile_ema: Decimal,

    // Additional liquidity metrics based on variance and volumes
    pub liquidity_real_sq_ema: Decimal, // For variance of liquidity
    pub liq_spread_ema: Decimal,        // Difference between up/down liquidity
    pub liq_ratio_ema: Decimal,         // Ratio of directional liquidity (liq_up / liq_dw)
    pub liq_total_ema: Decimal,         // Total directional liquidity (liq_up + liq_dw)
    pub volume_variance_ratio_ema: Decimal, // volume^2 / variance for stability metric
}

// SAFETY: MarketEmaState is repr(C), Copy, contains only Decimal (which is Pod with c-repr)
unsafe impl strategy_api::Pod for MarketEmaState {}

/// State for market_features_ema - fixed-size array of markets
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EmaState {
    pub markets: [MarketEmaState; NUM_INSTRUMENTS],
}

// SAFETY: EmaState is repr(C), Copy, contains only fixed-size arrays of Pod types
unsafe impl strategy_api::Pod for EmaState {}
