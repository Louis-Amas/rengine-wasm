use rengine_types::{
    Account, BookKey, ExecutionRequest, ExecutionType, Instrument, Order, OrderActions, OrderInfo,
    OrderReference, Side,
};
use std::collections::{BTreeMap, HashMap, HashSet};

pub fn reconcile_orders(
    key: &BookKey,
    asks: Vec<Order>,
    bids: Vec<Order>,
    current_orders: &HashMap<String, OrderInfo>,
    posts: &mut BTreeMap<Account, Vec<(Instrument, OrderInfo)>>,
    cancels: &mut BTreeMap<Account, Vec<(Instrument, OrderReference)>>,
) {
    let mut expected_orders: HashSet<OrderInfo> = HashSet::new();
    for order in asks {
        expected_orders.insert(OrderInfo::new(
            Side::Ask,
            order.price,
            order.size,
            order.tif,
        ));
    }
    for order in bids {
        expected_orders.insert(OrderInfo::new(
            Side::Bid,
            order.price,
            order.size,
            order.tif,
        ));
    }

    let mut current_orders_by_value: HashMap<_, _> = HashMap::new();
    for (id, open_order) in current_orders {
        current_orders_by_value.insert(open_order.clone(), id.clone());
    }

    for (id, open_order) in current_orders {
        if !expected_orders.contains(open_order) {
            let cancels = cancels.entry(key.account.clone()).or_default();
            cancels.push((
                key.instrument.clone(),
                OrderReference::ExternalOrderId(id.clone().into()),
            ));
        }
    }

    for expected in expected_orders {
        if !current_orders_by_value.contains_key(&expected) {
            let posts = posts.entry(key.account.clone()).or_default();
            posts.push((key.instrument.clone(), expected));
        }
    }
}

pub fn orders(
    posts: BTreeMap<Account, Vec<(Instrument, OrderInfo)>>,
    cancels: BTreeMap<Account, Vec<(Instrument, OrderReference)>>,
) -> Vec<ExecutionRequest> {
    posts
        .into_iter()
        .map(|(key, posts)| {
            ExecutionRequest::Orderbook(OrderActions::BulkPost((
                key,
                posts,
                ExecutionType::Managed,
            )))
        })
        .chain(cancels.into_iter().map(|(key, cancels)| {
            ExecutionRequest::Orderbook(OrderActions::BulkCancel((
                key,
                cancels,
                ExecutionType::Managed,
            )))
        }))
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;
    use rengine_types::{
        Account, AccountId, ExecutionRequest, Instrument, MarketType, Order, OrderActions,
        OrderInfo, Side, TimeInForce, Venue,
    };
    use rust_decimal_macros::dec;
    use std::collections::HashMap;

    #[test]
    fn test_reconcile_orders() {
        let venue: Venue = "venue".into();
        let account_id: AccountId = "test".into();
        let instrument: Instrument = "eth".into();
        let market_type = MarketType::Spot;

        let key = BookKey {
            account: Account {
                venue,
                market_type,
                account_id,
            },
            instrument: instrument.clone(),
        };

        let perp_open_orders: HashMap<_, _> = HashMap::from([
            (
                "42".to_string(),
                OrderInfo::new(Side::Ask, dec!(2000), dec!(1), TimeInForce::Unknown),
            ),
            (
                "43".to_string(),
                OrderInfo::new(Side::Ask, dec!(2100), dec!(1.5), TimeInForce::Unknown),
            ),
        ]);

        let asks = vec![Order {
            price: dec!(2100),
            size: dec!(1.5),
            tif: TimeInForce::PostOnly,
        }];
        let bids = vec![Order {
            price: dec!(1900),
            size: dec!(2),
            tif: TimeInForce::PostOnly,
        }];

        let mut posts = <_>::default();
        let mut cancels = <_>::default();

        reconcile_orders(
            &key,
            asks,
            bids,
            &perp_open_orders,
            &mut posts,
            &mut cancels,
        );

        let cancels_for_account = cancels
            .get(&key.account)
            .expect("expected cancels for account");

        assert_eq!(cancels_for_account.len(), 1);
        assert_eq!(
            cancels_for_account[0],
            (
                instrument.clone(),
                OrderReference::ExternalOrderId("42".into())
            )
        );

        let posts_for_account = posts.get(&key.account).expect("expected posts for account");
        assert_eq!(posts_for_account.len(), 1);
        assert_eq!(
            posts_for_account[0],
            (
                instrument.clone(),
                OrderInfo::new(Side::Bid, dec!(1900), dec!(2), TimeInForce::PostOnly)
            )
        );
    }

    #[test]
    fn test_reconcile_with_already_open_orders() {
        let venue: Venue = "venue".into();
        let account_id: AccountId = "test".into();
        let instrument: Instrument = "eth".into();
        let market_type = MarketType::Spot;

        let key = BookKey {
            account: Account {
                venue,
                market_type,
                account_id,
            },
            instrument: instrument.clone(),
        };

        let perp_open_orders: HashMap<_, _> = HashMap::from([
            (
                "42".to_string(),
                OrderInfo::new(Side::Ask, dec!(2000), dec!(1), TimeInForce::Unknown),
            ),
            (
                "43".to_string(),
                OrderInfo::new(Side::Ask, dec!(2100), dec!(1.5), TimeInForce::Unknown),
            ),
        ]);

        let asks = vec![];
        let bids = vec![];

        let mut posts = <_>::default();
        let mut cancels = <_>::default();

        reconcile_orders(
            &key,
            asks,
            bids,
            &perp_open_orders,
            &mut posts,
            &mut cancels,
        );

        let actions = orders(posts, cancels);

        // Find the BulkCancel action and compare IDs ignoring order
        let (acct, cancels) = actions
            .iter()
            .find_map(|a| match a {
                ExecutionRequest::Orderbook(OrderActions::BulkCancel((acct, cancels, _))) => {
                    Some((acct, cancels))
                }
                _ => None,
            })
            .expect("expected a BulkCancel action");

        assert_eq!(acct, &key.account);

        let mut got_ids: Vec<&str> = cancels
            .iter()
            .map(|(_, id)| match id {
                OrderReference::ExternalOrderId(id) => id.as_ref(),
                OrderReference::ClientOrderId(id) => id.as_ref(),
            })
            .collect();
        got_ids.sort_unstable();

        let mut expected_ids = vec!["42", "43"];
        expected_ids.sort_unstable();

        assert_eq!(got_ids, expected_ids);
    }
}
