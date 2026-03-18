use anyhow::Result;
use rengine_types::{
    BookKey, ClientOrderId, ExecutionRequest, ExecutionType, OrderActions, OrderInfo,
    OrderReference, Side, StateUpdateKey, StrategyConfiguration, TimeInForce,
};
use rust_decimal_macros::dec;
use std::{collections::HashSet, time::Duration};
use strategy_api::{
    bindings::export, get_book, get_open_orders, impl_guest_from_plugin, trace, Plugin,
};

struct ClientOrderIdStrategy;

#[derive(Default, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct State;

impl Plugin for ClientOrderIdStrategy {
    type State = State;

    fn init() -> StrategyConfiguration {
        let mut set = HashSet::new();
        set.insert(StateUpdateKey::UtcTimer {
            interval: Duration::from_secs(10),
        });
        StrategyConfiguration {
            triggers_keys: set,
            cooldown: None,
        }
    }

    fn execute(state: Self::State) -> Result<(Self::State, Vec<ExecutionRequest>), String> {
        let venue_book_key = "hyperliquid|eth/usdc-perp";
        let book = get_book(venue_book_key)?;

        let book_key_as_str = "hyperliquid|perp|hotwallet|eth/usdc-perp";
        let book_key: BookKey = book_key_as_str
            .parse()
            .map_err(|err: anyhow::Error| err.to_string())?;

        let open_orders = get_open_orders(book_key_as_str)?;

        trace!("Current open orders: {:?}", open_orders);

        let client_order_id_ask: ClientOrderId = "0x1234567890abcdef1234567890abcde1".into();
        let client_order_id_bid: ClientOrderId = "0x1234567890abcdef1234567890abcde2".into();

        let mut cancels = Vec::new();

        for (_id, info) in &open_orders {
            if info.client_order_id.as_ref() == Some(&client_order_id_ask) {
                cancels.push((
                    book_key.instrument.clone(),
                    OrderReference::ClientOrderId(client_order_id_ask.clone()),
                ));
            }
            if info.client_order_id.as_ref() == Some(&client_order_id_bid) {
                cancels.push((
                    book_key.instrument.clone(),
                    OrderReference::ClientOrderId(client_order_id_bid.clone()),
                ));
            }
        }

        if !cancels.is_empty() {
            trace!("Found existing orders, cancelling");
            let cancel = ExecutionRequest::Orderbook(OrderActions::BulkCancel((
                book_key.account,
                cancels,
                ExecutionType::Unmanaged,
            )));
            return Ok((state, vec![cancel]));
        }

        trace!("No existing orders with cloids, placing new orders");

        let usd_size = dec!(20);
        let spread_bp = dec!(0.0050);

        let ask_price = (book.top_ask.price * (dec!(1) + spread_bp))
            .round_dp_with_strategy(1, rust_decimal::RoundingStrategy::ToZero);
        let ask = OrderInfo::new(
            Side::Ask,
            ask_price,
            (usd_size / ask_price)
                .round_dp_with_strategy(4, rust_decimal::RoundingStrategy::ToZero),
            TimeInForce::PostOnly,
        )
        .with_client_order_id(client_order_id_ask);

        let bid_price = (book.top_bid.price * (dec!(1) - spread_bp))
            .round_dp_with_strategy(1, rust_decimal::RoundingStrategy::ToZero);
        let bid = OrderInfo::new(
            Side::Bid,
            bid_price,
            (usd_size / bid_price)
                .round_dp_with_strategy(4, rust_decimal::RoundingStrategy::ToZero),
            TimeInForce::PostOnly,
        )
        .with_client_order_id(client_order_id_bid);

        let post = ExecutionRequest::Orderbook(OrderActions::BulkPost((
            book_key.account,
            vec![
                (book_key.instrument.clone(), ask),
                (book_key.instrument, bid),
            ],
            ExecutionType::Unmanaged,
        )));

        Ok((state, vec![post]))
    }
}

impl_guest_from_plugin!(ClientOrderIdStrategy, "client_order_id_strategy");

export!(ClientOrderIdStrategy with_types_in strategy_api::bindings);
