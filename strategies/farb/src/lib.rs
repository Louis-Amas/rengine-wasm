use anyhow::Result;
use rengine_types::{
    BookKey, ExecutionRequest, Order, StateUpdateKey, StrategyConfiguration, TimeInForce,
    VenueBookKey,
};
use rust_decimal::{Decimal, RoundingStrategy};
use rust_decimal_macros::dec;
use std::collections::HashSet;
use strategy_api::{
    bindings::export,
    get_book, get_indicator, get_open_orders, get_perp_position, get_spot_exposure,
    impl_guest_from_plugin,
    orderbook::{orders, reconcile_orders},
    Plugin,
};

struct Farb;

#[derive(Default, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct State;

impl Plugin for Farb {
    type State = State;

    fn init() -> StrategyConfiguration {
        let mut set = HashSet::new();
        set.insert(StateUpdateKey::SetTopBook(VenueBookKey {
            venue: "hyperliquid".into(),
            instrument: "eth/usdc-spot".into(),
        }));

        set.insert(StateUpdateKey::SetTopBook(VenueBookKey {
            venue: "hyperliquid".into(),
            instrument: "eth/usdc-perp".into(),
        }));

        StrategyConfiguration {
            triggers_keys: set,
            cooldown: None,
        }
    }

    fn execute(state: Self::State) -> Result<(Self::State, Vec<ExecutionRequest>), String> {
        let pair = "eth/usdc";
        let venue = "hyperliquid";
        let spot_book_key = format!("{venue}|{pair}-spot");
        let perp_book_key = format!("{venue}|{pair}-perp");
        let account_id = "hotwallet";

        let spot_book = get_book(&spot_book_key)?;
        let perp_book = get_book(&perp_book_key)?;
        // funding rate
        let fr_key = "hyperliquid-funding-eth";
        let funding_rate = get_indicator(fr_key)?;
        let funding_rate_threshold = dec!(0.0000115); // 10% APY

        let spot_book_key_as_str = format!("{venue}|spot|hotwallet|{pair}-spot");
        let perp_book_key_as_str = format!("{venue}|perp|hotwallet|{pair}-perp");
        let spot_book_key: BookKey = spot_book_key_as_str
            .parse()
            .map_err(|e: anyhow::Error| e.to_string())?;
        let perp_book_key: BookKey = perp_book_key_as_str
            .parse()
            .map_err(|e: anyhow::Error| e.to_string())?;

        let mut spot_exposure =
            get_spot_exposure(&format!("{venue}|spot|{account_id}|eth")).unwrap_or(Decimal::ZERO);

        if spot_exposure > dec!(0) && spot_exposure < (dec!(5) * dec!(1) / spot_book.top_ask.price)
        {
            spot_exposure = dec!(0);
        }

        if spot_exposure < dec!(0) && spot_exposure > -(dec!(5) * dec!(1) / spot_book.top_ask.price)
        {
            spot_exposure = dec!(0);
        }

        // trace!("spot_exposure {spot_exposure}");

        let perp_position = get_perp_position(&format!("{venue}|perp|{account_id}|eth/usdc-perp"))
            .unwrap_or(Decimal::ZERO);

        // --- constants ---
        let almost_zero_quote = dec!(5); // small USD error tolerance
        let almost_zero = almost_zero_quote / perp_book.mid();
        // let clip_size_quote = dec!(20);
        // let clip_size_base = (Decimal::ONE / spot_book.top_bid.price * clip_size_quote)
        //     .round_dp_with_strategy(4, RoundingStrategy::ToZero);
        //
        let clip_size_base = dec!(0.0044);

        let max_exposure = clip_size_base;

        // trace!("clip size base {clip_size_base}");

        let (mut spot_bids, mut spot_asks) = (vec![], vec![]);
        let (mut perp_bids, mut perp_asks) = (vec![], vec![]);

        // --- funding-driven target exposure ---
        let target_exposure = if funding_rate.abs() >= funding_rate_threshold {
            if funding_rate > dec!(0) {
                // Expectation: Long base (via spot) and short perp
                clip_size_base
            } else {
                // Only arbitrage when funding rate is postive
                // Expectation: Short base (via spot) and long perp
                // -(clip_size_base)
                Decimal::ZERO
            }
        } else if funding_rate < dec!(0) {
            // unwind -> target exposure is flat
            Decimal::ZERO
        } else {
            return Ok((state, vec![]));
        };

        // trace!("target_exposure {target_exposure}");

        if spot_exposure.abs() <= max_exposure || target_exposure == Decimal::ZERO {
            let exposure_diff = (target_exposure - spot_exposure)
                .round_dp_with_strategy(4, RoundingStrategy::ToZero);

            if exposure_diff <= clip_size_base {
                if target_exposure == spot_exposure {
                    // exposure already equals target; do nothing
                } else if exposure_diff > almost_zero {
                    // need more base exposure (long)
                    // buy spot (maker bid)
                    spot_bids.push(Order {
                        size: clip_size_base,
                        price: spot_book.top_bid.price,
                        tif: TimeInForce::PostOnly,
                    });
                } else if exposure_diff < -almost_zero {
                    // need less base exposure (short)
                    // sell spot (maker ask)
                    spot_asks.push(Order {
                        size: clip_size_base,
                        price: spot_book.top_ask.price,
                        tif: TimeInForce::PostOnly,
                    });
                }
            }
        }

        let target_exposure = -target_exposure;
        if perp_position.abs() <= max_exposure {
            let exposure_diff = (target_exposure - perp_position)
                .round_dp_with_strategy(4, RoundingStrategy::ToZero);

            if target_exposure == perp_position {
                // exposure already equals target; do nothing
            } else if exposure_diff > almost_zero {
                // need more base exposure (long)
                // buy perp (maker bid)
                perp_bids.push(Order {
                    size: clip_size_base,
                    price: perp_book.top_bid.price,
                    tif: TimeInForce::PostOnly,
                });
            } else if exposure_diff < -almost_zero {
                // need less base exposure (short)
                // sell perp (maker ask)
                perp_asks.push(Order {
                    size: clip_size_base,
                    price: perp_book.top_ask.price,
                    tif: TimeInForce::PostOnly,
                });
            }
        }

        // --- reconciliation ---
        let mut posts = <_>::default();
        let mut cancels = <_>::default();

        let perp_open_orders = get_open_orders(&perp_book_key_as_str)?;
        reconcile_orders(
            &perp_book_key,
            perp_asks,
            perp_bids,
            &perp_open_orders,
            &mut posts,
            &mut cancels,
        );

        let spot_open_orders = get_open_orders(&spot_book_key_as_str)?;
        reconcile_orders(
            &spot_book_key,
            spot_asks,
            spot_bids,
            &spot_open_orders,
            &mut posts,
            &mut cancels,
        );

        Ok((state, orders(posts, cancels)))
    }
}

impl_guest_from_plugin!(Farb, "farb");

export!(Farb with_types_in strategy_api::bindings);
