use crate::{
    types::*,
    ws::{ActionResponse, ExtraData, Message, PostResponse},
};
use anyhow::{bail, Result};
use chrono::Utc;
use rengine_metrics::latencies::{record_latency, LatencyId};
use rengine_non_wasm_types::{send_changes, ChangesTx};
use rengine_types::{
    Account, Action, BookKey, ExecutionResult, Mapping, OpenOrder, OrderInfo, TimeInForce,
    Timestamp, TimestampedData,
};
use rust_decimal::Decimal;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, info, warn};

pub(crate) struct HyperLiquidPrivateStreamer {
    account: Account,
    instrument_mapping: Mapping,
    changes_tx: ChangesTx,
}

fn order_update_to_action(account: Account, order: OrderUpdate) -> Option<Action> {
    let status = order.status;
    let order = order.order;
    let size = order.sz;
    let key = BookKey {
        account,
        instrument: order.coin,
    };

    match status {
        OrderStatus::Open => Some(Action::SetOpenOrder(
            key,
            order.oid.to_string(),
            OpenOrder {
                info: OrderInfo {
                    side: order.side.into(),
                    price: order.limit_px,
                    size,
                    tif: TimeInForce::Unknown,
                    client_order_id: order.cloid,
                    order_type: Default::default(),
                },
                original_size: order.orig_sz,
                is_snapshot: true,
            },
        )),
        OrderStatus::Filled => Some(Action::UpdateOpenOrder(key, order.oid.to_string(), size)),
        OrderStatus::Canceled => Some(Action::RemoveOpenOrder(key, order.oid.to_string())),
        _ => None,
    }
}

impl HyperLiquidPrivateStreamer {
    pub(crate) const fn new(
        account: Account,
        instrument_mapping: Mapping,
        changes_tx: ChangesTx,
    ) -> Self {
        Self {
            account,
            instrument_mapping,
            changes_tx,
        }
    }

    fn handle_order_updates(&self, updates: OrderUpdates) -> Result<()> {
        let results: Vec<Action> = updates
            .data
            .into_iter()
            .filter_map(|order| order_update_to_action(self.account.clone(), order))
            .collect();

        send_changes(&self.changes_tx, results);

        Ok(())
    }

    fn position_value(trade: &Trade) -> Result<Decimal> {
        match trade.dir {
            Direction::OpenLong | Direction::CloseShort | Direction::ShortGreaterLong => {
                Ok(trade.start_position + trade.sz)
            }
            Direction::CloseLong | Direction::OpenShort | Direction::LongGreaterShort => {
                Ok(trade.start_position - trade.sz)
            }
            _ => bail!("couldn't decode position dir {:?}", trade.dir),
        }
    }

    fn handle_user(&self, user: User) -> Result<()> {
        match user.data {
            UserData::Fills(fills) => {
                if fills.is_empty() {
                    return Ok(());
                }

                let time = Utc::now();
                let mut trades = Vec::with_capacity(fills.len());
                let mut last_set: Option<Action> = None;

                for fill in &fills {
                    let instrument = self
                        .instrument_mapping
                        .map_instrument(&self.account.venue, &fill.coin)?;

                    if self.account.market_type != instrument.market_type {
                        continue;
                    }

                    let book_key = BookKey {
                        account: self.account.clone(),
                        instrument: fill.coin.clone(),
                    };

                    let trade = fill.to_engine_trade(&instrument, self.account.clone(), time);
                    trades.push((book_key.clone(), trade));

                    if fill.dir.is_perp() {
                        let value = Self::position_value(fill)?;
                        last_set = Some(Action::SetPerpPosition(book_key, value));
                    }
                }

                let mut actions = vec![Action::RecordTrades(trades)];
                if let Some(set_position) = last_set {
                    actions.push(set_position);
                }

                send_changes(&self.changes_tx, actions);
            }
            UserData::Funding(funding) => {
                info!(?funding, "received funding");
            }
        }
        Ok(())
    }

    pub(crate) async fn handle_connection(
        self,
        mut receiver: UnboundedReceiver<(Message, Option<ExtraData>)>,
    ) -> Result<()> {
        while let Some((msg, extra_data)) = receiver.recv().await {
            match msg {
                Message::OrderUpdates(updates) => {
                    if let Err(err) = self.handle_order_updates(updates) {
                        warn!(?err);
                    }
                }
                Message::User(user) => {
                    if let Err(err) = self.handle_user(user) {
                        warn!(?err);
                    }
                }
                Message::Post(data) => {
                    let PostResponse::Action { payload } = data.data.response;

                    let action = match payload.response {
                        ActionResponse::Order { data } => {
                            let Some(extra_data) = extra_data else {
                                warn!(?extra_data, "missing extra data for orders with msg");
                                continue;
                            };

                            match extra_data {
                                ExtraData::Orders(emited_at, orders, instant) => {
                                    record_latency(LatencyId::HyperLiquidOrderPlace, instant);
                                    let results = data.bulk_post_results(orders);
                                    Action::HandleExecutionResult(ExecutionResult::Orderbook(
                                        TimestampedData {
                                            data: results,
                                            emited_at,
                                            received_at: Timestamp::now(),
                                        },
                                    ))
                                }
                                _ => {
                                    warn!(
                                        "received extra data which is not order for order response"
                                    );
                                    continue;
                                }
                            }
                        }
                        ActionResponse::Cancel { data } => {
                            let Some(extra_data) = extra_data else {
                                warn!(?data, "missing extra data for cancels with msg");
                                continue;
                            };

                            match extra_data {
                                ExtraData::Cancel(emited_at, cancels, instant) => {
                                    record_latency(LatencyId::HyperLiquidOrderCancel, instant);
                                    let results = data.bulk_cancel_results(cancels);
                                    Action::HandleExecutionResult(ExecutionResult::Orderbook(
                                        TimestampedData {
                                            data: results,
                                            emited_at,
                                            received_at: Timestamp::now(),
                                        },
                                    ))
                                }
                                _ => {
                                    warn!(
                                        "received extra data which is not cancel for cancel response"
                                    );
                                    continue;
                                }
                            }
                        }
                    };

                    send_changes(&self.changes_tx, vec![action]);
                }
                Message::Unknown | Message::Pong | Message::SubscriptionResponse => {}
                _ => {
                    error!(?msg, "couldn't handle message");
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        private::ws::order_update_to_action,
        types::{BasicOrder, HyperLiquidSide, OrderStatus, OrderUpdate},
    };
    use rengine_types::{Account, Action, BookKey, MarketType};
    use rust_decimal_macros::dec;

    #[test]
    fn test_order_updates_action_update_order() {
        let order = OrderUpdate {
            order: BasicOrder {
                coin: "ETH".into(),
                side: HyperLiquidSide::Bid,
                sz: dec!(0.5),
                oid: 0,
                // cloid: None,
                orig_sz: dec!(1),
                limit_px: dec!(4000),
                cloid: None,
                // timestamp: 0,
            },
            status: OrderStatus::Filled,
            // status_timestamp: 0,
        };

        let account = Account {
            venue: "test".into(),
            account_id: "test".into(),
            market_type: MarketType::Spot,
        };

        let Action::UpdateOpenOrder(book_key, id, size) =
            order_update_to_action(account.clone(), order).unwrap()
        else {
            panic!("not a filled order");
        };

        assert_eq!(
            book_key,
            BookKey {
                account,
                instrument: "ETH".into(),
            }
        );
        assert_eq!(id, 0.to_string());
        assert_eq!(size, dec!(0.5));
    }
}
