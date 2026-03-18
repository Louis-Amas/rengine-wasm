use crate::{
    execution::{Actions, Signature},
    types::{HyperliquidTrades, L2Book, OrderUpdates, User},
};
use alloy::primitives::Address;
use anyhow::{anyhow, Result};
use futures::{SinkExt, StreamExt};
use rengine_types::{
    BulkCancelResult, BulkCancelStatus, BulkPostResult, BulkPostStatus, Instrument, OrderInfo,
    OrderReference, OrderbookResults, Symbol, Timestamp,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    self,
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::{
    select,
    sync::{broadcast, mpsc},
    time,
    time::{sleep, Instant as InstantTokio},
};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tracing::{error, info, warn};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub(crate) enum Subscription {
    // AllMids,
    Trades { coin: Symbol },
    L2Book { coin: Symbol },
    UserEvents { user: Address },
    // UserFills { user: Address },
    // Candle { coin: String, interval: String },
    OrderUpdates { user: Address },
    // UserFundings { user: Address },
    // UserNonFundingLedgerUpdates { user: Address },
    // Notification { user: Address },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RequestType {
    #[serde(rename = "action")]
    Action,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PostRequest {
    #[serde(rename = "type")]
    pub request_type: RequestType,
    pub payload: Payload,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Payload {
    pub action: Actions,
    pub nonce: u64,
    pub signature: Signature,
    // pub vaultAddress: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "method", rename_all = "lowercase")]
pub(crate) enum WsHyperliquidMessage {
    Ping,
    Subscribe { subscription: Subscription },
    Post { id: u64, request: PostRequest },
}

#[derive(Deserialize, Clone, Debug)]
pub(crate) struct PostData {
    pub(crate) data: PostResponseWrapper,
}

#[derive(Deserialize, Clone, Debug)]
pub(crate) struct PostResponseWrapper {
    pub(crate) id: u64,
    pub(crate) response: PostResponse,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum PostResponse {
    Action { payload: ActionPayload },
}

#[derive(Deserialize, Clone, Debug)]
pub(crate) struct ActionPayload {
    pub response: ActionResponse,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OrderDataStatus {
    WaitingForFill,
    WaitingForTrigger,
    Error(String),
    Resting(RestingOrder),
    Filled(FilledOrder),
}

impl From<OrderDataStatus> for BulkPostStatus {
    fn from(value: OrderDataStatus) -> Self {
        match value {
            OrderDataStatus::Filled(order) => Self::Filled {
                order_id: order.oid.to_string(),
                size: order.total_sz,
                average_price: order.avg_px,
            },
            OrderDataStatus::Resting(order) => Self::Resting {
                order_id: order.oid.to_string(),
            },
            OrderDataStatus::Error(err_msg) => Self::Error(err_msg),
            _ => Self::Error("Unknown".to_string()),
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CancelDataStatus {
    Success,
    Error(String),
}

impl From<CancelDataStatus> for BulkCancelStatus {
    fn from(value: CancelDataStatus) -> Self {
        match value {
            CancelDataStatus::Success => Self::Success,
            CancelDataStatus::Error(err) => Self::Error(err),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub(crate) struct ExchangeDataStatuses<T> {
    pub(crate) statuses: Vec<T>,
}

impl ExchangeDataStatuses<OrderDataStatus> {
    pub(crate) fn bulk_post_results(
        self,
        orders: Vec<(Instrument, OrderInfo)>,
    ) -> OrderbookResults {
        let stats = self.statuses.into_iter().map(BulkPostStatus::from);
        let paired: Vec<_> = orders
            .into_iter()
            .zip(stats)
            .map(|((instrument, order), status)| BulkPostResult {
                instrument,
                order,
                status,
            })
            .collect();
        OrderbookResults::BulkPost(paired)
    }
}

impl ExchangeDataStatuses<CancelDataStatus> {
    pub(crate) fn bulk_cancel_results(
        self,
        cancels: Vec<(Instrument, OrderReference)>,
    ) -> OrderbookResults {
        let stats = self.statuses.into_iter().map(BulkCancelStatus::from);
        let paired: Vec<_> = cancels
            .into_iter()
            .zip(stats)
            .map(|((instrument, order_ref), status)| BulkCancelResult {
                instrument,
                order_id: order_ref,
                status,
            })
            .collect();
        OrderbookResults::BulkCancel(paired)
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum ActionResponse {
    Cancel {
        data: ExchangeDataStatuses<CancelDataStatus>,
    },
    Order {
        data: ExchangeDataStatuses<OrderDataStatus>,
    },
}

#[derive(Deserialize, Debug, Clone)]
pub(crate) struct RestingOrder {
    pub(crate) oid: u64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilledOrder {
    pub(crate) total_sz: Decimal,
    pub(crate) avg_px: Decimal,
    pub(crate) oid: u64,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "channel")]
#[serde(rename_all = "camelCase")]
pub(crate) enum Message {
    NoData,
    Post(PostData),
    SubscriptionResponse,
    // HyperliquidError(String),
    // AllMids(AllMids),
    Trades(HyperliquidTrades),
    L2Book(L2Book),
    User(User),
    // UserFills(UserFills),
    // Candle(Candle),
    // SubscriptionResponse,
    OrderUpdates(OrderUpdates),
    // UserFundings(UserFundings),
    // UserNonFundingLedgerUpdates(UserNonFundingLedgerUpdates),
    // Notification(Notification),
    Pong,
    #[serde(other)]
    Unknown,
}

#[derive(Debug)]
pub(crate) enum ExtraData {
    Orders(Timestamp, Vec<(Symbol, OrderInfo)>, Instant),
    Cancel(Timestamp, Vec<(Symbol, OrderReference)>, Instant),
}

const WS_URL: &str = "wss://api.hyperliquid.xyz/ws";

pub(crate) struct WsClient {
    subscriptions: Vec<Subscription>,
    reconnect_signal_tx: broadcast::Sender<()>,
    incoming_message_sender: mpsc::UnboundedSender<(Message, Option<ExtraData>)>,
    outcoming_message_receiver: mpsc::UnboundedReceiver<(WsHyperliquidMessage, Option<ExtraData>)>,
    ids_data: HashMap<u64, ExtraData>,
    idle_timeout: Duration,
}

impl WsClient {
    pub(crate) fn new(
        subscriptions: Vec<Subscription>,
        reconnect_signal_tx: broadcast::Sender<()>,
        incoming_message_sender: mpsc::UnboundedSender<(Message, Option<ExtraData>)>,
        outcoming_message_receiver: mpsc::UnboundedReceiver<(
            WsHyperliquidMessage,
            Option<ExtraData>,
        )>,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            subscriptions,
            reconnect_signal_tx,
            incoming_message_sender,
            outcoming_message_receiver,
            ids_data: Default::default(),
            idle_timeout,
        }
    }

    pub(crate) async fn run(mut self) -> Result<()> {
        let subs_msgs: Vec<String> = self
            .subscriptions
            .into_iter()
            .map(|sub| {
                let msg = WsHyperliquidMessage::Subscribe { subscription: sub };
                serde_json::to_string(&msg)
            })
            .collect::<Result<_, _>>()
            .map_err(|err| anyhow!(err))?;

        let ping_msg = serde_json::to_string(&WsHyperliquidMessage::Ping)?;
        loop {
            info!("[hyperliquid] try to connect to stream");
            self.ids_data.clear();
            if let Err(err) = self.reconnect_signal_tx.send(()) {
                error!(
                    "[hyperliquid] catastrophic failure couldn't send reconnection signal, {err:?}"
                );
            }
            let (mut stream, _) = match connect_async(WS_URL).await {
                Ok(stream) => stream,
                Err(err) => {
                    error!("couldn't connect to hyperliquid stream {err:?}");
                    time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            info!("[hyperliquid] connected stream");
            for msg in &subs_msgs {
                if let Err(err) = stream.send(WsMessage::text(msg)).await {
                    warn!("couldn't send ws message {err:?}");
                }
            }
            info!("[hyperliquid] subscribed to all channel");

            let mut ticker = time::interval(Duration::from_secs(10));

            let mut idle_timer = Box::pin(sleep(self.idle_timeout));

            loop {
                select! {
                    _ = &mut idle_timer => {
                        warn!("[hyperliquid] no message received for {:?}, reconnecting...", self.idle_timeout);
                        break;
                    }
                    Some(Ok(msg)) = stream.next() => {
                        match msg {
                            WsMessage::Text(msg) => {
                                let parsed = match serde_json::from_str::<Message>(&msg) {
                                    Ok(msg) => msg,
                                    Err(err) => {
                                        warn!("[hyperliquid] couldn't parse msg {msg} {err:?}");
                                        continue;
                                    }
                                };
                                idle_timer.as_mut().reset(InstantTokio::now() + self.idle_timeout);


                                let extra_data = match &parsed {
                                    Message::Post(data) => {
                                        let id = data.data.id;

                                        self.ids_data.remove(&id)
                                    }
                                    Message::Unknown => {
                                        warn!("[hyperliquid] couldn't parse msg {msg}");
                                        continue;
                                    },
                                    _ => None
                                };


                                if let Err(err) = self.incoming_message_sender.send((parsed, extra_data)) {
                                    error!("[hyperliquid] catastrophic error sending message {err:?}");
                                }
                            }
                            WsMessage::Ping(_) | WsMessage::Pong(_) => {
                                // Ignore for now, could log or handle
                            }
                            _ => {
                                warn!("[hyperliquid] received something weird, reconnecting...");
                            }
                        }
                    },
                    Some((original_msg, extra_data)) = self.outcoming_message_receiver.recv() => {

                        let Ok(msg) = serde_json::to_string(&original_msg) else {
                            warn!("[hyperliquid] couldn't serialize msg {original_msg:?}");
                            continue;
                        };

                        if let Err(err) = stream.send(WsMessage::Text(msg)).await {
                            warn!("[hyperliquid] couldn't send ws message {err:?}");
                        }

                        if let WsHyperliquidMessage::Post { id, request: _ } = original_msg {
                            if let Some(extra_data) = extra_data {
                                self.ids_data.insert(id, extra_data);
                            }
                        }
                    }
                    _ = ticker.tick() => {
                        if let Err(err) = stream.send(WsMessage::Text(ping_msg.clone())).await {
                            error!("[hyperliquid] failed to send ping: {err:?}");
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::{Message, Subscription};
    use crate::ws::{
        ActionResponse, CancelDataStatus, OrderDataStatus, PostResponse, WsHyperliquidMessage,
    };
    use alloy::primitives::address;

    #[test]
    fn test_serialize_subscription() {
        let sub = Subscription::L2Book { coin: "ETH".into() };

        let sub_str = serde_json::to_string(&sub).unwrap();

        println!("{sub_str}");

        let sub = WsHyperliquidMessage::Subscribe { subscription: sub };

        let sub_str = serde_json::to_string(&sub).unwrap();

        println!("{sub_str}");
    }

    #[test]
    fn test_serialize_subscription_user() {
        let sub = Subscription::UserEvents {
            user: address!("0xa6F1eF0733FC462627f280053040f5f47D7fA0c6"),
        };

        let sub_str = serde_json::to_string(&sub).unwrap();

        println!("{sub_str}");

        let sub = WsHyperliquidMessage::Subscribe { subscription: sub };

        let sub_str = serde_json::to_string(&sub).unwrap();

        println!("{sub_str}");
    }

    #[test]
    fn test_deserialize_post_cancel_success() {
        let raw = r#"
        {
            "channel": "post",
            "data": {
                "id": 137,
                "response": {
                    "type": "action",
                    "payload": {
                        "status": "ok",
                        "response": {
                            "type": "cancel",
                            "data": {
                                "statuses": ["success", "success", "success"]
                            }
                        }
                    }
                }
            }
        }
        "#;

        let message: Message = serde_json::from_str(raw).expect("Failed to deserialize");

        match message {
            Message::Post(post) => {
                assert_eq!(post.data.id, 137);
                match &post.data.response {
                    PostResponse::Action { payload } => {
                        let ActionResponse::Cancel { data } = &payload.response else {
                            panic!("not working");
                        };
                        assert_eq!(data.statuses.len(), 3);

                        data.statuses
                            .iter()
                            .for_each(|status| assert_eq!(status, &CancelDataStatus::Success));
                    }
                }
            }
            _ => panic!("Expected Message::Post"),
        }
    }

    #[test]
    fn test_deserialize_post_order_response() {
        let raw = r#"
    {
        "channel": "post",
        "data": {
            "id": 1,
            "response": {
                "type": "action",
                "payload": {
                    "status": "ok",
                    "response": {
                        "type": "order",
                        "data": {
                            "statuses": [
                                {
                                    "resting": {
                                        "oid": 102250555965
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        }
    }
    "#;

        let message: Message = serde_json::from_str(raw).expect("Failed to deserialize");

        match message {
            Message::Post(post) => {
                assert_eq!(post.data.id, 1);
                match &post.data.response {
                    PostResponse::Action { payload } => match &payload.response {
                        ActionResponse::Order { data } => {
                            assert_eq!(data.statuses.len(), 1);
                            let OrderDataStatus::Resting(order) = &data.statuses[0] else {
                                panic!("is not a resting order");
                            };
                            assert_eq!(order.oid, 102250555965);
                        }
                        _ => panic!("Expected order response"),
                    },
                }
            }
            _ => panic!("Expected Message::Post"),
        }
    }
}
