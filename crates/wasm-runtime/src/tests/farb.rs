use crate::{strategy::StrategyRuntime, Runtime};
use parking_lot::RwLock;
use rengine_types::{
    BalanceKey, BookKey, ExecutionRequest, Level, MarketType, OrderActions, Side, State,
    TopBookUpdate, VenueBookKey,
};
use rust_decimal_macros::dec;
use std::sync::Arc;

const STRATEGY_BYTES: &[u8] = include_bytes!("../../../../strategies-wasm/farb.cwasm");

#[test]
fn wasm_strategy_farb() {
    let state: Arc<RwLock<State>> = Default::default();
    let mut runtime = Runtime::new(state.clone()).unwrap();
    let strategy = runtime.instantiate_strategy(STRATEGY_BYTES).unwrap();

    // 0.0000125 * 100 * 24 * 365
    let mut guard = state.write();

    guard
        .indicators
        .insert("hyperliquid-funding-eth".into(), dec!(0.0000125));

    guard.book.insert(
        VenueBookKey {
            venue: "hyperliquid".into(),
            instrument: "eth/usdc-spot".into(),
        },
        TopBookUpdate {
            top_ask: Level {
                size: dec!(5),
                price: dec!(4000),
            },
            top_bid: Level {
                size: dec!(5),
                price: dec!(3995),
            },
        },
    );

    guard.book.insert(
        VenueBookKey {
            venue: "hyperliquid".into(),
            instrument: "eth/usdc-perp".into(),
        },
        TopBookUpdate {
            top_ask: Level {
                size: dec!(5),
                price: dec!(3995),
            },
            top_bid: Level {
                size: dec!(5),
                price: dec!(3990),
            },
        },
    );
    drop(guard);

    let (_, result) = runtime.execute(&strategy, &[], None).unwrap();

    let mut spot_req = None;
    let mut perp_req = None;

    for req in result.requests {
        match req {
            ExecutionRequest::Orderbook(OrderActions::BulkPost((account, orders, _))) => {
                if account.market_type == MarketType::Spot {
                    spot_req = Some((account, orders));
                } else if account.market_type == MarketType::Perp {
                    perp_req = Some((account, orders));
                }
            }
            _ => panic!("Unexpected request type"),
        }
    }

    let (spot_account, spot_orders) = spot_req.expect("missing spot request");
    let (perp_account, perp_orders) = perp_req.expect("missing perp request");

    // --- check spot ---
    assert_eq!(spot_account.venue, "hyperliquid");
    assert_eq!(spot_account.market_type, MarketType::Spot);
    assert_eq!(spot_account.account_id, "hotwallet");

    let (spot_symbol, spot_order) = spot_orders.into_iter().next().unwrap();
    assert_eq!(spot_symbol, "eth/usdc-spot");
    assert_eq!(spot_order.side, Side::Bid);
    assert_eq!(spot_order.price, dec!(3995));
    assert_eq!(spot_order.size, dec!(0.0044));

    // --- check perp ---
    assert_eq!(perp_account.venue, "hyperliquid");
    assert_eq!(perp_account.market_type, MarketType::Perp);
    assert_eq!(perp_account.account_id, "hotwallet");

    let (perp_symbol, perp_order) = perp_orders.into_iter().next().unwrap();
    assert_eq!(perp_symbol, "eth/usdc-perp");
    assert_eq!(perp_order.side, Side::Ask);
    assert_eq!(perp_order.price, dec!(3995));
    assert_eq!(perp_order.size, dec!(0.0044));

    let spot_exposure_key: BalanceKey = "hyperliquid|spot|hotwallet|eth".parse().unwrap();
    let position_key: BookKey = "hyperliquid|perp|hotwallet|eth/usdc-perp".parse().unwrap();

    let mut guard = state.write();
    guard.spot_exposures.insert(spot_exposure_key, dec!(0.0054));
    guard.positions.insert(position_key, -dec!(0.0044));
    drop(guard);

    let (_, result) = runtime.execute(&strategy, &[], None).unwrap();
    assert!(result.requests.is_empty());

    let mut guard = state.write();
    guard
        .indicators
        .insert("hyperliquid-funding-eth".into(), -dec!(0.00001));
    drop(guard);

    let (_, result) = runtime.execute(&strategy, &[], None).unwrap();

    let mut spot_req = None;
    let mut perp_req = None;

    for req in result.requests {
        match req {
            ExecutionRequest::Orderbook(OrderActions::BulkPost((account, orders, _))) => {
                if account.market_type == MarketType::Spot {
                    spot_req = Some((account, orders));
                } else if account.market_type == MarketType::Perp {
                    perp_req = Some((account, orders));
                }
            }
            _ => panic!("Unexpected request type"),
        }
    }

    let (spot_account, spot_orders) = spot_req.expect("missing spot request");
    let (perp_account, perp_orders) = perp_req.expect("missing perp request");

    // --- check spot ---
    assert_eq!(spot_account.venue, "hyperliquid");
    assert_eq!(spot_account.market_type, MarketType::Spot);
    assert_eq!(spot_account.account_id, "hotwallet");

    let (spot_symbol, spot_order) = spot_orders.into_iter().next().unwrap();
    assert_eq!(spot_symbol, "eth/usdc-spot");
    assert_eq!(spot_order.side, Side::Ask);
    assert_eq!(spot_order.price, dec!(4000));
    assert_eq!(spot_order.size, dec!(0.00440));

    // --- check perp ---
    assert_eq!(perp_account.venue, "hyperliquid");
    assert_eq!(perp_account.market_type, MarketType::Perp);
    assert_eq!(perp_account.account_id, "hotwallet");

    let (perp_symbol, perp_order) = perp_orders.into_iter().next().unwrap();
    assert_eq!(perp_symbol, "eth/usdc-perp");
    assert_eq!(perp_order.side, Side::Bid);
    assert_eq!(perp_order.price, dec!(3990));
    assert_eq!(perp_order.size, dec!(0.0044));

    let mut guard = state.write();
    guard.positions.clear();
    guard.spot_exposures.clear();
    drop(guard);

    let (_, result) = runtime.execute(&strategy, &[], None).unwrap();

    assert!(result.requests.is_empty());
}
