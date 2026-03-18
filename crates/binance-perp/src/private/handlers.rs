use crate::private::types::{
    AccountUpdateData, BinanceExecutionType, BinanceOrderStatus, OrderTradeUpdateData,
};
use chrono::{DateTime, TimeZone, Utc};
use rengine_types::{
    identifiers::{Account, BalanceKey, BookKey},
    order::{OpenOrder, OrderInfo},
    primitive::{MarketType, Timestamp},
    state::Action,
    trade::Trade,
    Instrument, Mapping, Symbol,
};

/// Handle account update events (balance and position updates)
pub fn handle_account_update(
    _event_time: DateTime<Utc>,
    update_data: AccountUpdateData,
    account: Account,
) -> Vec<Action> {
    let mut actions = vec![];

    for bal in update_data.balances {
        let symbol: Symbol = bal.asset.to_string().into();

        let key = BalanceKey {
            account: account.clone(),
            symbol,
        };
        actions.push(Action::SetBalance(key, bal.wallet_balance));
    }

    for pos in update_data.positions {
        let exchange_symbol: Instrument = pos.symbol.clone().into();
        let key = BookKey {
            account: account.clone(),
            instrument: exchange_symbol,
        };
        actions.push(Action::SetPerpPosition(key, pos.position_amount));
    }

    actions
}

/// Handle order and trade update events
pub fn handle_order_trade_update(
    _event_time: DateTime<Utc>,
    order: OrderTradeUpdateData,
    account: Account,
    mapping: &Mapping,
) -> Vec<Action> {
    let mut actions = vec![];

    let exchange_symbol: Instrument = order.symbol.clone().into();

    let book_key = BookKey {
        account: account.clone(),
        instrument: exchange_symbol.clone(),
    };

    // Handle Order Update
    let order_info = OrderInfo::new(
        order.side.clone().into(),
        order.original_price,
        order.original_quantity,
        order.time_in_force.into(),
    )
    .with_client_order_id(order.client_order_id.clone().into());

    let open_order = OpenOrder {
        info: order_info,
        original_size: order.original_quantity,
        is_snapshot: false,
    };

    match order.order_status {
        BinanceOrderStatus::New | BinanceOrderStatus::PartiallyFilled => {
            actions.push(Action::SetOpenOrder(
                book_key.clone(),
                order.order_id.to_string(),
                open_order,
            ));
        }
        BinanceOrderStatus::Filled
        | BinanceOrderStatus::Canceled
        | BinanceOrderStatus::Expired
        | BinanceOrderStatus::ExpiredInMatch => {
            actions.push(Action::RemoveOpenOrder(
                book_key.clone(),
                order.order_id.to_string(),
            ));
        }
    }

    // Handle Trade
    if order.execution_type == BinanceExecutionType::Trade {
        if let Ok(details) = mapping.map_instrument(&account.venue, &exchange_symbol) {
            let emitted_at = chrono::Utc
                .timestamp_millis_opt(order.trade_time.timestamp_millis())
                .single()
                .map(rengine_types::Timestamp::from)
                .unwrap_or_else(rengine_types::Timestamp::now);

            let trade = Trade {
                emitted_at,
                received_at: Timestamp::now(),
                order_id: order.order_id as i64,
                trade_id: order.trade_id as i64,
                account,
                base: details.base.clone(),
                quote: details.quote,
                side: order.side.into(),
                market_type: MarketType::Perp,
                price: order.last_filled_price,
                size: order.last_filled_quantity,
                fee: order.commission.unwrap_or_default(),
                fee_symbol: order.commission_asset.unwrap_or_default().into(),
            };
            actions.push(Action::RecordTrades(vec![(book_key, trade)]));
        } else {
            tracing::error!("Failed to map instrument {}", exchange_symbol);
        }
    }

    actions
}
