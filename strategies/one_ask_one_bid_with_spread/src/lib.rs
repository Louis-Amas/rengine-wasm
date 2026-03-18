use anyhow::Result;
use rengine_types::{
    BookKey, ExecutionRequest, Order, StateUpdateKey, StrategyConfiguration, TimeInForce,
    VenueBookKey,
};
use rust_decimal_macros::dec;
use std::collections::HashSet;
use strategy_api::{
    bindings::export,
    get_book, get_open_orders, impl_guest_from_plugin,
    orderbook::{orders, reconcile_orders},
    trace, Plugin,
};

struct OneAskOneBidWithSpread;

#[derive(Default, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct State;

impl Plugin for OneAskOneBidWithSpread {
    type State = State;

    fn init() -> StrategyConfiguration {
        let mut set = HashSet::new();
        set.insert(StateUpdateKey::SetTopBook(VenueBookKey {
            venue: "hyperliquid".into(),
            instrument: "eth/usdc-spot".into(),
        }));

        StrategyConfiguration {
            triggers_keys: set,
            cooldown: None,
        }
    }

    fn execute(state: Self::State) -> Result<(Self::State, Vec<ExecutionRequest>), String> {
        let venue_book_key = "hyperliquid|eth/usdc-spot";

        let book = get_book(venue_book_key)?;

        let usd_size = dec!(20);
        let spread_in_usd = dec!(1000);

        let ask_price = book.top_ask.price + spread_in_usd;
        let ask = Order {
            size: (usd_size / ask_price)
                .round_dp_with_strategy(4, rust_decimal::RoundingStrategy::ToZero),
            price: ask_price,

            tif: TimeInForce::PostOnly,
        };

        let bid_price = book.top_bid.price - spread_in_usd;
        let bid = Order {
            size: (usd_size / bid_price)
                .round_dp_with_strategy(4, rust_decimal::RoundingStrategy::ToZero),
            price: bid_price,
            tif: TimeInForce::PostOnly,
        };

        let book_key_as_str = "hyperliquid|spot|hotwallet|eth/usdc-spot";
        let book_key: BookKey = book_key_as_str
            .parse()
            .map_err(|err: anyhow::Error| err.to_string())?;

        let open_orders = get_open_orders(book_key_as_str)?;

        let mut posts = <_>::default();
        let mut cancels = <_>::default();

        reconcile_orders(
            &book_key,
            vec![ask],
            vec![bid],
            &open_orders,
            &mut posts,
            &mut cancels,
        );

        let orders = orders(posts, cancels);

        trace!("orders {orders:?}");

        Ok((state, orders))
    }
}

impl_guest_from_plugin!(OneAskOneBidWithSpread, "one_ask_one_bid_with_spread");

export!(OneAskOneBidWithSpread with_types_in strategy_api::bindings);
