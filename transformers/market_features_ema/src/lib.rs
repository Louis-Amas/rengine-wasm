use anyhow::Result;
use market_features_types::{get_market_id, EmaState, MarketEmaState, MarketState, INSTRUMENTS};
use rengine_types::{ExecutionRequest, StateUpdateKey, StrategyConfiguration};
use rust_decimal_macros::dec;
use std::{collections::HashSet, time::Duration};
use strategy_api::{export, impl_guest_from_unsafe_plugin, UnsafePlugin};

struct EmaVolume;

impl UnsafePlugin for EmaVolume {
    type State = EmaState;

    fn init() -> StrategyConfiguration {
        let mut keys = HashSet::new();
        // Run every 60 seconds
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

            // Get current values from market_features using FromIndicators
            let mf = MarketState::from_indicators_with_market(&market_id);

            // Work directly on Pod state - no conversion needed
            let ema_state = &mut state.markets[idx];

            // Process EMA for this market
            process_ema(ema_state, &mf);

            // Collect indicators with market prefix
            requests.extend(ema_state.indicators_with_market(&market_id));
        }

        Ok(requests)
    }
}

fn process_ema(state: &mut MarketEmaState, mf: &MarketState) {
    // Calculate EMAs with alpha = 0.01 (matching Python ema parameter)
    let alpha = dec!(0.01);

    // Initialize defaults if zero (first run)
    if state.variance_ema == dec!(0) {
        state.variance_ema = dec!(0.0000000001);
    }
    if state.variance_felt_ema == dec!(0) {
        state.variance_felt_ema = dec!(0.0000000001);
    }
    if state.variance2_ema == dec!(0) {
        state.variance2_ema = dec!(0.0000000001);
    }
    if state.variation_ema == dec!(0) {
        state.variation_ema = dec!(0.00001);
    }
    if state.slippage_ema == dec!(0) {
        state.slippage_ema = dec!(0.00001);
    }
    if state.slippage_sq_ema == dec!(0) {
        state.slippage_sq_ema = dec!(0.0000000001);
    }
    if state.var_up_ema == dec!(0) {
        state.var_up_ema = dec!(0.00000001);
    }
    if state.var_dw_ema == dec!(0) {
        state.var_dw_ema = dec!(0.00000001);
    }

    // Calculate deltas
    state.micro_cluster_count_delta = mf.micro_cluster_count - state.micro_cluster_count_last;
    state.log_return_delta = mf.log_return - state.log_return_last;
    state.variation_delta = mf.variation - state.variation_last;
    state.variance_delta = mf.variance - state.variance_last;
    let variance_felt_delta = dec!(0) - state.variance_felt_last; // variance_felt not in MarketState
    let skew_delta = mf.skew - state.skew_last;
    state.volume_delta = mf.volume - state.volume_last;
    let volume_retail_delta = mf.volume_retail - state.volume_retail_last;
    let volume_retail_up_delta = mf.volume_retail_up - state.volume_retail_up_last;
    let volume_retail_down_delta = mf.volume_retail_down - state.volume_retail_down_last;
    let antiselek_delta = mf.antiselek - state.antiselek_last;
    let slippage_delta = mf.slippage - state.slippage_last;
    let slippage_sq_delta = mf.slippage_sq - state.slippage_sq_last;

    let micro_price_hot_variance_delta =
        mf.micro_price_hot_variance - state.micro_price_hot_variance_last;
    let micro_price_warm_variance_delta =
        mf.micro_price_warm_variance - state.micro_price_warm_variance_last;
    let micro_price_cold_variance_delta =
        mf.micro_price_cold_variance - state.micro_price_cold_variance_last;
    let mean_reversion_delta = mf.mean_reversion - state.mean_reversion_last;
    let mean_reversion_hot_delta = mf.mean_reversion_hot - state.mean_reversion_hot_last;
    let mean_reversion_warm_delta = mf.mean_reversion_warm - state.mean_reversion_warm_last;
    let mean_reversion_cold_delta = mf.mean_reversion_cold - state.mean_reversion_cold_last;
    let pnl_hot_delta = mf.pnl_hot - state.pnl_hot_last;
    let pnl_warm_delta = mf.pnl_warm - state.pnl_warm_last;
    let pnl_cold_delta = mf.pnl_cold - state.pnl_cold_last;

    let flow_up_delta = mf.flow_up - state.flow_up_last;
    let flow_dw_delta = mf.flow_dw - state.flow_dw_last;
    let size_up_delta = mf.size_up - state.size_up_last;
    let size_dw_delta = mf.size_dw - state.size_dw_last;
    let var_up_delta = mf.var_up - state.var_up_last;
    let var_dw_delta = mf.var_dw - state.var_dw_last;

    // Update EMAs: ema += alpha * (delta - ema)
    state.micro_cluster_count_ema +=
        alpha * (state.micro_cluster_count_delta - state.micro_cluster_count_ema);
    state.log_return_ema += alpha * (state.log_return_delta - state.log_return_ema);
    state.variance_ema += alpha * (state.variance_delta - state.variance_ema);
    state.variance_felt_ema += alpha * (variance_felt_delta - state.variance_felt_ema);
    state.variance2_ema +=
        alpha * (state.log_return_delta * state.log_return_delta - state.variance2_ema);
    state.variation_ema += alpha * (state.variation_delta - state.variation_ema);
    state.skew_ema += alpha * (skew_delta - state.skew_ema);

    state.volume_ema += alpha * (state.volume_delta - state.volume_ema);
    state.volume_retail_ema += alpha * (volume_retail_delta - state.volume_retail_ema);
    state.volume_retail_up_ema += alpha * (volume_retail_up_delta - state.volume_retail_up_ema);
    state.volume_retail_down_ema +=
        alpha * (volume_retail_down_delta - state.volume_retail_down_ema);

    // trade_flow uses current value, not delta
    state.trade_flow_ema += alpha * (mf.trade_flow - state.trade_flow_ema);
    state.trade_flow_retail_ema += alpha * (mf.trade_flow_retail - state.trade_flow_retail_ema);

    state.slippage_ema += alpha * (slippage_delta - state.slippage_ema);
    state.slippage_sq_ema += alpha * (slippage_sq_delta - state.slippage_sq_ema);
    state.antiselek_ema += alpha * (antiselek_delta - state.antiselek_ema);

    state.micro_price_hot_variance_ema +=
        alpha * (micro_price_hot_variance_delta - state.micro_price_hot_variance_ema);
    state.micro_price_warm_variance_ema +=
        alpha * (micro_price_warm_variance_delta - state.micro_price_warm_variance_ema);
    state.micro_price_cold_variance_ema +=
        alpha * (micro_price_cold_variance_delta - state.micro_price_cold_variance_ema);
    state.mean_reversion_ema += alpha * (mean_reversion_delta - state.mean_reversion_ema);
    state.mean_reversion_hot_ema +=
        alpha * (mean_reversion_hot_delta - state.mean_reversion_hot_ema);
    state.mean_reversion_warm_ema +=
        alpha * (mean_reversion_warm_delta - state.mean_reversion_warm_ema);
    state.mean_reversion_cold_ema +=
        alpha * (mean_reversion_cold_delta - state.mean_reversion_cold_ema);
    state.pnl_hot_ema += alpha * (pnl_hot_delta - state.pnl_hot_ema);
    state.pnl_warm_ema += alpha * (pnl_warm_delta - state.pnl_warm_ema);
    state.pnl_cold_ema += alpha * (pnl_cold_delta - state.pnl_cold_ema);

    state.flow_up_ema += alpha * (flow_up_delta - state.flow_up_ema);
    state.flow_dw_ema += alpha * (flow_dw_delta - state.flow_dw_ema);
    state.size_up_ema += alpha * (size_up_delta - state.size_up_ema);
    state.size_dw_ema += alpha * (size_dw_delta - state.size_dw_ema);
    state.var_up_ema += alpha * (var_up_delta - state.var_up_ema);
    state.var_dw_ema += alpha * (var_dw_delta - state.var_dw_ema);

    // Ratio EMAs (computed from other EMAs)
    state.liquidity_real_ema = if state.variance_ema > dec!(0) {
        state.volume_ema / state.variance_ema
    } else {
        dec!(0)
    };

    state.liq_up_ema = if state.var_up_ema > dec!(0) {
        state.flow_up_ema / state.var_up_ema
    } else {
        dec!(0)
    };

    state.liq_dw_ema = if state.var_dw_ema > dec!(0) {
        state.flow_dw_ema / state.var_dw_ema
    } else {
        dec!(0)
    };

    state.momentum_ema = if state.variance_ema > dec!(0) {
        state.variance2_ema / state.variance_ema
    } else {
        dec!(0)
    };

    // Power calculations
    let slip = if state.variation_ema > dec!(0) {
        state.slippage_ema / state.variation_ema
    } else {
        dec!(0)
    };

    if slip < dec!(1) {
        state.power_ema = slip / (dec!(1) - slip);
    }

    let slip_sq = if state.variance_ema > dec!(0) {
        state.slippage_sq_ema / state.variance_ema
    } else {
        dec!(0)
    };

    if slip_sq < dec!(1) {
        state.power2_ema = dec!(2) * slip_sq / (dec!(1) - slip_sq);
    }

    // Q-learning PNL calculations
    state.q_pnl_ema = dec!(0.00005) * state.volume_ema
        - state.liquidity_real_ema * state.skew_ema / (state.power_ema + dec!(1));
    state.q_pnl2_ema =
        state.volume_ema * (dec!(0.00005) - state.variation_ema / (state.power_ema + dec!(1)));
    state.q_perf_ema = if state.volume_ema > dec!(0) {
        state.q_pnl_ema / state.volume_ema
    } else {
        dec!(0)
    };

    // liquidity_imp uses current value, not delta
    state.liquidity_imp_ema += alpha * (mf.liquidity_imp - state.liquidity_imp_ema);
    state.liquidity_imbalance_ema +=
        alpha * (mf.liquidity_imbalance - state.liquidity_imbalance_ema);

    // Smile calculation
    state.smile_ema = state.var_up_ema - state.var_dw_ema;

    // Additional liquidity metrics based on variance and volumes
    // Liquidity squared EMA for measuring liquidity variance/stability
    let liquidity_real_sq = state.liquidity_real_ema * state.liquidity_real_ema;
    state.liquidity_real_sq_ema += alpha * (liquidity_real_sq - state.liquidity_real_sq_ema);

    // Liquidity spread: difference between up and down directional liquidity
    state.liq_spread_ema = state.liq_up_ema - state.liq_dw_ema;

    // Liquidity ratio: up/down directional liquidity ratio
    state.liq_ratio_ema = if state.liq_dw_ema > dec!(0) {
        state.liq_up_ema / state.liq_dw_ema
    } else if state.liq_up_ema > dec!(0) {
        dec!(100) // Cap at 100 when denominator is zero but numerator exists
    } else {
        dec!(1) // Neutral when both are zero
    };

    // Total directional liquidity
    state.liq_total_ema = state.liq_up_ema + state.liq_dw_ema;

    // Volume-variance ratio: higher means more stable liquidity
    state.volume_variance_ratio_ema = if state.variance_ema > dec!(0) {
        let vol_sq = state.volume_ema * state.volume_ema;
        vol_sq / state.variance_ema
    } else {
        dec!(0)
    };

    // Update last values
    state.micro_cluster_count_last = mf.micro_cluster_count;
    state.log_return_last = mf.log_return;
    state.variation_last = mf.variation;
    state.variance_last = mf.variance;
    state.variance_felt_last = dec!(0); // variance_felt not in MarketState
    state.skew_last = mf.skew;
    state.volume_last = mf.volume;
    state.volume_retail_last = mf.volume_retail;
    state.volume_retail_up_last = mf.volume_retail_up;
    state.volume_retail_down_last = mf.volume_retail_down;
    state.antiselek_last = mf.antiselek;
    state.slippage_last = mf.slippage;
    state.slippage_sq_last = mf.slippage_sq;
    state.micro_price_hot_variance_last = mf.micro_price_hot_variance;
    state.micro_price_warm_variance_last = mf.micro_price_warm_variance;
    state.micro_price_cold_variance_last = mf.micro_price_cold_variance;
    state.mean_reversion_last = mf.mean_reversion;
    state.mean_reversion_hot_last = mf.mean_reversion_hot;
    state.mean_reversion_warm_last = mf.mean_reversion_warm;
    state.mean_reversion_cold_last = mf.mean_reversion_cold;
    state.pnl_hot_last = mf.pnl_hot;
    state.pnl_warm_last = mf.pnl_warm;
    state.pnl_cold_last = mf.pnl_cold;
    state.size_up_last = mf.size_up;
    state.size_dw_last = mf.size_dw;
    state.flow_up_last = mf.flow_up;
    state.flow_dw_last = mf.flow_dw;
    state.var_up_last = mf.var_up;
    state.var_dw_last = mf.var_dw;
}

impl_guest_from_unsafe_plugin!(EmaVolume, "market_features_ema");

export!(EmaVolume with_types_in strategy_api::bindings);
