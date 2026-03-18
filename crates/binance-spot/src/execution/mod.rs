pub mod types;

use crate::execution::types::{
    ActionPayload, BinanceCancelOrderReq, BinanceCreateOrderReq, BinanceIncomingMsg,
    BinanceOrderStatus, BinanceWsRequest, ExtraData,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{pkcs8::DecodePrivateKey, Signer, SigningKey};
use frunk_ws::{
    engine::{bind_stream, run_ws_loop},
    handler::to_handler,
    handlers::forwarder::{forward_messages, ForwarderState, JsonParser},
    types::{ContextState, HandlerOutcome},
};
use futures::{FutureExt, SinkExt};
use rengine_interfaces::ExchangeExecution;
use rengine_types::{
    state::Action, BulkCancelResult, BulkCancelStatus, BulkPostResult, BulkPostStatus,
    ExecutionResult, Instrument, OrderInfo, OrderReference, OrderbookResults, Timestamp,
    TimestampedData,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    env,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_tungstenite::tungstenite::Message;
use tracing::error;

type ChangesTx = mpsc::Sender<Vec<Action>>;

pub struct BinanceSpotExecutor {
    api_key: String,
    signer: SigningKey,
    request_tx: mpsc::UnboundedSender<String>,
    pending_requests: Arc<Mutex<HashMap<String, ExtraData>>>,
}

const WS_URL: &str = "wss://ws-api.binance.com:443/ws-api/v3";

impl BinanceSpotExecutor {
    pub fn new(api_key: String, secret_key: String, changes_tx: ChangesTx) -> Result<Self> {
        let (incoming_msgs_tx, incoming_msgs_rx) = mpsc::unbounded_channel::<BinanceIncomingMsg>();
        let (internal_request_tx, internal_request_rx) = mpsc::unbounded_channel::<String>();
        let request_stream = UnboundedReceiverStream::new(internal_request_rx);

        let pending_requests = Arc::new(Mutex::new(HashMap::new()));
        let pending_requests_clone = pending_requests.clone();

        let url = env::var("BINANCE_SPOT_WS_API_URL").unwrap_or_else(|_| WS_URL.to_string());

        tokio::spawn(async move {
            let url = url;

            let forwarder_state = ForwarderState {
                sender: incoming_msgs_tx.clone(),
            };
            let context_state = ContextState::new("BinanceSpotExecution");
            let state = frunk::hlist![forwarder_state, context_state];

            static PARSER: JsonParser<BinanceIncomingMsg> = JsonParser::new();

            let handler = to_handler(move |ws, state, msg| {
                forward_messages(ws, state, msg, &PARSER, |_| false)
            });

            let request_stream = bind_stream(request_stream, |ws, _state, msg| {
                async move {
                    if let Err(e) = ws.send(Message::Text(msg)).await {
                        error!("Failed to send request: {}", e);
                    }
                    HandlerOutcome::Continue
                }
                .boxed()
            });

            if let Err(e) = run_ws_loop(url, state, vec![], handler, vec![request_stream]).await {
                error!("Execution WS loop failed: {}", e);
            }
        });

        tokio::spawn(async move {
            let mut rx = incoming_msgs_rx;
            while let Some(msg) = rx.recv().await {
                let id = msg.event_id();
                let extra_data = {
                    let mut pending = pending_requests_clone.lock().unwrap();
                    pending.remove(&id)
                };

                if let Some(extra_data) = extra_data {
                    let action = match msg {
                        BinanceIncomingMsg::Success { result, .. } => match extra_data {
                            ExtraData::Order(instrument, order) => {
                                let status = if result.status == BinanceOrderStatus::Filled {
                                    let average_price = if result.executed_qty.is_zero() {
                                        result.price
                                    } else {
                                        result.cummulative_quote_qty / result.executed_qty
                                    };
                                    BulkPostStatus::Filled {
                                        order_id: result.order_id.to_string(),
                                        size: result.executed_qty,
                                        average_price,
                                    }
                                } else {
                                    BulkPostStatus::Resting {
                                        order_id: result.order_id.to_string(),
                                    }
                                };

                                let bulk_result = BulkPostResult {
                                    instrument,
                                    order,
                                    status,
                                };
                                Some(Action::HandleExecutionResult(ExecutionResult::Orderbook(
                                    TimestampedData {
                                        data: OrderbookResults::BulkPost(vec![bulk_result]),
                                        emited_at: Timestamp::from(result.transact_time),
                                        received_at: Timestamp::now(),
                                    },
                                )))
                            }
                            ExtraData::Cancel(instrument, order_ref) => {
                                let result = BulkCancelResult {
                                    instrument,
                                    order_id: order_ref,
                                    status: BulkCancelStatus::Success,
                                };
                                Some(Action::HandleExecutionResult(ExecutionResult::Orderbook(
                                    TimestampedData {
                                        data: OrderbookResults::BulkCancel(vec![result]),
                                        emited_at: Timestamp::now(),
                                        received_at: Timestamp::now(),
                                    },
                                )))
                            }
                        },
                        BinanceIncomingMsg::Error { error, .. } => match extra_data {
                            ExtraData::Order(instrument, order) => {
                                let result = BulkPostResult {
                                    instrument,
                                    order,
                                    status: BulkPostStatus::Error(error.msg),
                                };
                                Some(Action::HandleExecutionResult(ExecutionResult::Orderbook(
                                    TimestampedData {
                                        data: OrderbookResults::BulkPost(vec![result]),
                                        emited_at: Timestamp::now(),
                                        received_at: Timestamp::now(),
                                    },
                                )))
                            }
                            ExtraData::Cancel(instrument, order_ref) => {
                                let result = BulkCancelResult {
                                    instrument,
                                    order_id: order_ref,
                                    status: BulkCancelStatus::Error(error.msg),
                                };
                                Some(Action::HandleExecutionResult(ExecutionResult::Orderbook(
                                    TimestampedData {
                                        data: OrderbookResults::BulkCancel(vec![result]),
                                        emited_at: Timestamp::now(),
                                        received_at: Timestamp::now(),
                                    },
                                )))
                            }
                        },
                    };

                    if let Some(action) = action {
                        if let Err(e) = changes_tx.send(vec![action]).await {
                            error!("Failed to send execution result: {}", e);
                        }
                    }
                } else {
                    tracing::warn!("Received response for unknown request: {}", id);
                }
            }
        });

        let pem_bytes = B64
            .decode(secret_key)
            .context("decoding base64 PEM content")?;
        let pem = String::from_utf8(pem_bytes).context("converting PEM bytes to UTF-8 string")?;
        let signer = SigningKey::from_pkcs8_pem(&pem).context("parsing Ed25519 PKCS#8 PEM")?;

        Ok(Self {
            api_key,
            signer,
            request_tx: internal_request_tx,
            pending_requests,
        })
    }

    fn sign<T: Serialize>(&self, payload: &T) -> Result<String> {
        let obj = serde_json::to_value(payload)?;
        let mut params = BTreeMap::new();
        if let Value::Object(obj) = obj {
            params.extend(obj.into_iter().filter_map(|(k, v)| {
                if v.is_null() {
                    None
                } else {
                    Some((k, v.to_string().trim_matches('"').to_string()))
                }
            }));
        }
        let query_str = serde_urlencoded::to_string(&params)?;
        Ok(B64.encode(self.signer.sign(query_str.as_bytes()).to_bytes()))
    }
}

#[async_trait]
impl ExchangeExecution for BinanceSpotExecutor {
    async fn post_orders(&self, orders: Vec<(Instrument, OrderInfo)>) -> Result<()> {
        for (instrument, order) in orders {
            let mut req = BinanceCreateOrderReq::from_order(
                instrument.clone(),
                order.clone(),
                self.api_key.clone(),
            );
            let signature = self.sign(&req)?;
            req.signature = Some(signature);

            let id = uuid::Uuid::new_v4().to_string();
            let payload = BinanceWsRequest::new(id.clone(), ActionPayload::Place(req));

            {
                let mut pending = self.pending_requests.lock().unwrap();
                pending.insert(id, ExtraData::Order(instrument, order));
            }

            let msg = serde_json::to_string(&payload)?;
            self.request_tx
                .send(msg)
                .map_err(|_| anyhow!("Channel closed"))?;
        }
        Ok(())
    }

    async fn cancel_orders(&self, cancels: Vec<(Instrument, OrderReference)>) -> Result<()> {
        for (instrument, order_ref) in cancels {
            let mut req = BinanceCancelOrderReq::from_cancel(
                instrument.clone(),
                order_ref.clone(),
                self.api_key.clone(),
            );
            let signature = self.sign(&req)?;
            req.signature = Some(signature);

            let id = uuid::Uuid::new_v4().to_string();
            let payload = BinanceWsRequest::new(id.clone(), ActionPayload::Cancel(req));

            {
                let mut pending = self.pending_requests.lock().unwrap();
                pending.insert(id, ExtraData::Cancel(instrument, order_ref));
            }

            let msg = serde_json::to_string(&payload)?;
            self.request_tx
                .send(msg)
                .map_err(|_| anyhow!("Channel closed"))?;
        }
        Ok(())
    }

    fn max_response_duration(&self) -> Duration {
        Duration::from_millis(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rengine_types::{Side, TimeInForce};
    use rengine_utils::init_logging;
    use rust_decimal_macros::dec;

    #[tokio::test]
    #[ignore]
    async fn test_binance_spot_executor() {
        init_logging();

        let api_key = std::env::var("BINANCE_API_KEY").unwrap_or_default();
        let secret_key = std::env::var("BINANCE_SECRET_KEY").unwrap_or_default();
        let (changes_tx, _) = mpsc::channel(100);
        let executor = BinanceSpotExecutor::new(api_key, secret_key, changes_tx).unwrap();

        let instrument = Instrument::from("BTCUSDC");
        let order = OrderInfo {
            side: Side::Bid,
            price: dec!(80000),
            size: dec!(0.0001),
            tif: TimeInForce::PostOnly,
            client_order_id: None,
            order_type: Default::default(),
        };

        dbg!(&order);

        executor
            .post_orders(vec![(instrument, order)])
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_binance_spot_cancel() {
        init_logging();

        let api_key = std::env::var("BINANCE_API_KEY").unwrap_or_default();
        let secret_key = std::env::var("BINANCE_SECRET_KEY").unwrap_or_default();
        let (changes_tx, _) = mpsc::channel(100);
        let executor = BinanceSpotExecutor::new(api_key, secret_key, changes_tx).unwrap();

        let instrument = Instrument::from("BTCUSDC");
        let client_order_id = "test_order_12345";
        let order = OrderInfo {
            side: Side::Ask,
            price: dec!(95000),
            size: dec!(0.0001),
            tif: TimeInForce::PostOnly,
            client_order_id: Some(client_order_id.to_string().into()),
            order_type: Default::default(),
        };

        dbg!(&order);

        executor
            .post_orders(vec![(instrument.clone(), order)])
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(5)).await;

        executor
            .cancel_orders(vec![(
                instrument,
                OrderReference::ClientOrderId(client_order_id.to_string().into()),
            )])
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
