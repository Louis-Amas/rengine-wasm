//! **Market Features Transformer** — tick-level microstructure feature extraction.
//!
//! Triggered on [`SetTradeFlow`](rengine_types::StateUpdateKey::SetTradeFlow) for each
//! active instrument. On every invocation it processes the latest batch of public trades
//! and updates the top-of-book snapshot.
//!
//! **Inputs**: `get_trade_flow()` (all recent trades), `get_book(instrument)` (best bid/ask).
//!
//! **Outputs**: `mf_{market_id}_{field}` indicators via [`ToIndicators`](rengine_macros::ToIndicators).
//!
//! ## Micro-cluster model
//!
//! Trades sharing the same millisecond timestamp are grouped into a "micro-cluster" —
//! these represent a single taker order that matched multiple resting orders. The cluster
//! is finalized when a trade with a new timestamp arrives. Per-cluster statistics (slippage,
//! geometric mean, directional flow) are more meaningful than per-trade statistics because
//! they correspond to single execution decisions.
//!
//! ## Parabolic market model
//!
//! The transformer tracks three micro-price estimators (hot/warm/cold) using a parabolic
//! market model with parameter α = 2. These model different mean-reversion timescales
//! and produce PnL and mean-reversion signals at each scale.

use anyhow::Result;
use market_features_types::{get_market_id, MarketState, State, INSTRUMENTS};
use rengine_types::{ExecutionRequest, Side, StateUpdateKey, StrategyConfiguration, VenueBookKey};
use rust_decimal::{prelude::ToPrimitive, Decimal, MathematicalOps};
use rust_decimal_macros::dec;
use std::collections::HashSet;
use strategy_api::{export, get_book, get_trade_flow, impl_guest_from_unsafe_plugin, UnsafePlugin};

struct MarketFeatures;

/// Contract size multiplier. Set to 1 for spot and standard perp markets.
const CONTRACT_SIZE: Decimal = dec!(1);
/// Parabolic market model parameter (α). Controls the decay rate of micro-price estimators.
/// β = α/(1+α) ≈ 0.667 is the corresponding EMA weight.
const ALPHA: Decimal = dec!(2);

impl UnsafePlugin for MarketFeatures {
    type State = State;

    fn init() -> StrategyConfiguration {
        let mut keys = HashSet::new();
        for instrument in INSTRUMENTS {
            keys.insert(StateUpdateKey::SetTradeFlow(instrument.parse().unwrap()));
            // keys.insert(StateUpdateKey::SetTopBook(instrument.parse().unwrap()));
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

            // Work directly on Pod state - no conversion needed!
            let market_state = &mut state.markets[idx];

            // Process trade flow updates
            if let Some(trades) = all_trades.get(&instrument_key) {
                if !trades.is_empty() {
                    process_trade_flow(market_state, trades)?;
                }
            }

            // Process top of book updates
            if let Ok(book) = get_book(instrument) {
                process_top_of_book(market_state, &book)?;
            }

            // Collect indicators with market prefix
            requests.extend(market_state.indicators_with_market(&market_id));
        }

        Ok(requests)
    }
}

/// Processes a batch of public trades for a single instrument.
///
/// Trades are grouped into micro-clusters by timestamp. Within each cluster the function
/// accumulates size, volume, and log returns. When a new timestamp is encountered the
/// previous cluster is finalized:
///
/// 1. **Slippage**: size-weighted geometric mean of within-cluster log returns.
/// 2. **Adverse selection** (`antiselek`): volume × (|total_log| - |geom_ratio|).
/// 3. **Micro-price updates**: three exponential estimators (hot/warm/cold) using the
///    parabolic model parameter α = 2.
/// 4. **Mean reversion**: cross-product of micro-price level and cluster log return.
/// 5. **PnL**: simulated market-making PnL at each timescale.
/// 6. **Higher moments**: variation (|log|), variance (log²), skew (|log³|).
/// 7. **Directional flow**: size/volume/variance split by taker side.
/// 8. **Retail detection**: trades with zero log return (price unchanged).
fn process_trade_flow(
    state: &mut MarketState,
    trades: &[rengine_types::PublicTrade],
) -> Result<(), String> {
    let mut last_event_time_u64 = state.last_event_time.to_u64().unwrap_or(0);

    for trade in trades {
        let size = trade.size;
        let price = trade.price;
        let event_time = trade.time;
        let vol = size * price * CONTRACT_SIZE;

        state.volume += vol;

        // Calculate log return
        let log = if state.last_price > dec!(0) && price > dec!(0) {
            (price / state.last_price).checked_ln().unwrap_or(dec!(0))
        } else {
            dec!(0)
        };

        state.log_return += log;

        // Check if same event time (part of same micro-cluster)
        if event_time == last_event_time_u64 && last_event_time_u64 > 0 {
            // Same micro-cluster
            state.micro_cluster_log += log;
            state.micro_cluster_size += size;
            state.micro_cluster_volume += vol;
            state.micro_cluster_geom += size * state.micro_cluster_log;
            state.micro_cluster_geom_sq +=
                size * state.micro_cluster_log.checked_powi(2).unwrap_or(dec!(0));
        } else {
            // New micro-cluster - process previous one
            if state.micro_cluster_size > dec!(0) {
                state.micro_cluster_count += dec!(1);

                // Calculate micro liquidity
                if state.micro_cluster_log != dec!(0) {
                    let log_sq = state.micro_cluster_log.checked_powi(2).unwrap_or(dec!(0));
                    if log_sq > dec!(0) {
                        state.micro_liquidity = state.micro_cluster_volume / log_sq;
                    }
                }

                // Slippage metrics
                let geom_ratio = if state.micro_cluster_size > dec!(0) {
                    state.micro_cluster_geom / state.micro_cluster_size
                } else {
                    dec!(0)
                };
                state.slippage += geom_ratio.abs();
                state.slippage_sq += state.micro_cluster_geom_sq / state.micro_cluster_size;
                state.antiselek +=
                    state.micro_cluster_volume * (state.micro_cluster_log.abs() - geom_ratio.abs());

                // Beta for parabolic market
                let beta = ALPHA / (dec!(1) + ALPHA);

                // Mean reversion calculations (before updating micro prices)
                state.mean_reversion_hot += state.micro_price_hot * state.micro_cluster_log;
                state.mean_reversion_warm += state.micro_price_warm * state.micro_cluster_log;
                state.mean_reversion_cold += state.micro_price_cold * state.micro_cluster_log;

                // Calculate increments
                let increment_hot = beta * state.micro_cluster_log;
                let increment_warm = beta * (state.micro_cluster_log - state.micro_price_warm);
                let increment_cold = if state.micro_cluster_log - state.micro_price_cold > dec!(0) {
                    beta * (state.micro_cluster_log - state.micro_price_cold)
                } else {
                    (dec!(1) - beta) * (state.micro_cluster_log - state.micro_price_cold)
                };

                // Update micro prices
                state.micro_price_hot += increment_hot;
                state.micro_price_warm += increment_warm;
                state.micro_price_cold += increment_cold;

                // Update variances
                state.micro_price_hot_variance += increment_hot.checked_powi(2).unwrap_or(dec!(0));
                state.micro_price_warm_variance +=
                    increment_warm.checked_powi(2).unwrap_or(dec!(0));
                state.micro_price_cold_variance +=
                    increment_cold.checked_powi(2).unwrap_or(dec!(0));

                // Update PnLs
                state.pnl_hot -=
                    state.micro_cluster_log * (state.micro_cluster_log - state.micro_price_hot);
                state.pnl_warm -=
                    state.micro_cluster_log * (state.micro_cluster_log - state.micro_price_warm);
                state.pnl_cold -=
                    state.micro_cluster_log * (state.micro_cluster_log - state.micro_price_cold);

                // Higher moments
                state.variation += state.micro_cluster_log.abs();
                state.variance += state.micro_cluster_log.checked_powi(2).unwrap_or(dec!(0));
                state.skew += state
                    .micro_cluster_log
                    .checked_powi(3)
                    .unwrap_or(dec!(0))
                    .abs();

                // Alternative mean reversion calculation
                let w = (ALPHA + dec!(2)) / ALPHA;
                let inc_warm_sq = increment_warm.checked_powi(2).unwrap_or(dec!(0));
                let mcl_sq = state.micro_cluster_log.checked_powi(2).unwrap_or(dec!(0));
                state.mean_reversion +=
                    dec!(0.5) * (ALPHA / (ALPHA + dec!(1))) * (w * inc_warm_sq - mcl_sq);

                // Flow direction (buyer is taker = Ask side in your Python)
                let is_buyer_taker = trade.side == Side::Ask;

                if is_buyer_taker {
                    state.size_up += state.micro_cluster_size;
                    state.flow_up += state.micro_cluster_volume;
                    state.var_up += mcl_sq;
                    state.trade_flow += state.micro_cluster_volume;
                    state.smile += mcl_sq;
                } else {
                    state.size_dw += state.micro_cluster_size;
                    state.flow_dw += state.micro_cluster_volume;
                    state.var_dw += mcl_sq;
                    state.trade_flow -= state.micro_cluster_volume;
                    state.smile -= mcl_sq;
                }
            }

            // Reset for new micro-cluster
            state.micro_cluster_log = log;
            state.micro_cluster_size = size;
            state.micro_cluster_volume = vol;
            state.micro_cluster_geom = size * log;
            state.micro_cluster_geom_sq = size * log * log;
        }

        // Retail flow detection (price not moving)
        if log == dec!(0) {
            state.volume_retail += vol;
            let is_buyer_taker = trade.side == Side::Ask;
            if is_buyer_taker {
                state.trade_flow_retail += vol;
                state.volume_retail_down += vol;
            } else {
                state.trade_flow_retail -= vol;
                state.volume_retail_up += vol;
            }
        }

        state.last_price = price;
        last_event_time_u64 = event_time;
    }

    state.last_event_time = Decimal::from(last_event_time_u64);

    Ok(())
}

/// Updates order-book-derived features from the current best bid/ask.
///
/// Computed features:
/// - **spread**: `ln(best_ask / best_bid)` — tighter is more liquid.
/// - **liquidity_imp**: `(bid_notional + ask_notional) / (2 × spread²)` — depth relative to spread.
/// - **liquidity_imbalance**: `bid_notional - ask_notional` — positive means heavier bid.
/// - **liquidity_real**: `volume / variance` — variance-normalized cumulative liquidity.
/// - **liq_up / liq_dw**: directional liquidity as `flow / variance` per side.
fn process_top_of_book(
    state: &mut MarketState,
    book: &rengine_types::TopBookUpdate,
) -> Result<(), String> {
    let best_bid_price = book.top_bid.price;
    let best_ask_price = book.top_ask.price;
    let best_bid_size = book.top_bid.size;
    let best_ask_size = book.top_ask.size;

    // Calculate spread
    state.spread = if best_bid_price > dec!(0) && best_ask_price > dec!(0) {
        (best_ask_price / best_bid_price)
            .checked_ln()
            .unwrap_or(dec!(0))
    } else {
        dec!(0)
    };

    // Calculate liquidity_imp
    state.liquidity_imp = if state.spread > dec!(0) {
        let spread_sq = state.spread.checked_powi(2).unwrap_or(dec!(0));
        if spread_sq > dec!(0) {
            (best_bid_size * best_bid_price + best_ask_size * best_ask_price)
                / (dec!(2) * spread_sq)
        } else {
            dec!(0)
        }
    } else {
        dec!(0)
    };

    // Calculate liquidity imbalance
    state.liquidity_imbalance = best_bid_size * best_bid_price - best_ask_size * best_ask_price;

    // Calculate liquidity based on variance and volumes
    state.liquidity_real = if state.variance > dec!(0) {
        state.volume / state.variance
    } else {
        dec!(0)
    };

    state.liq_up = if state.var_up > dec!(0) {
        state.flow_up / state.var_up
    } else {
        dec!(0)
    };

    state.liq_dw = if state.var_dw > dec!(0) {
        state.flow_dw / state.var_dw
    } else {
        dec!(0)
    };

    Ok(())
}

impl_guest_from_unsafe_plugin!(MarketFeatures, "market_features");

export!(MarketFeatures with_types_in strategy_api::bindings);
