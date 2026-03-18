pub mod types;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
use chrono::{TimeZone, Utc};
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
use std::{
    collections::HashMap,
    env,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_tungstenite::tungstenite::Message;
use tracing::error;
use types::{
    ActionPayload, BinanceIncomingMsg, BinanceOrderResponse, BinancePerpCancelOrderReq,
    BinancePerpCreateOrderReq, BinanceWsRequest, ExtraData,
};

type ChangesTx = mpsc::Sender<Vec<Action>>;

pub struct BinancePerpExecutor {
    api_key: String,
    signer: SigningKey,
    request_tx: mpsc::UnboundedSender<String>,
    pending_requests: Arc<Mutex<HashMap<String, ExtraData>>>,
}

const WS_URL: &str = "wss://ws-fapi.binance.com/ws-fapi/v1";

impl BinancePerpExecutor {
    pub fn new(api_key: String, secret_key: String, changes_tx: ChangesTx) -> Result<Self> {
        let (incoming_msgs_tx, incoming_msgs_rx) = mpsc::unbounded_channel::<BinanceIncomingMsg>();
        let (internal_request_tx, internal_request_rx) = mpsc::unbounded_channel::<String>();
        let request_stream = UnboundedReceiverStream::new(internal_request_rx);

        let pending_requests = Arc::new(Mutex::new(HashMap::new()));
        let pending_requests_clone = pending_requests.clone();

        let url = env::var("BINANCE_PERP_WS_API_URL").unwrap_or_else(|_| WS_URL.to_string());

        tokio::spawn(async move {
            let url = url;

            let forwarder_state = ForwarderState {
                sender: incoming_msgs_tx.clone(),
            };
            let context_state = ContextState::new("BinancePerpExecution");
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
                                match serde_json::from_value::<BinanceOrderResponse>(result) {
                                    Ok(response) => {
                                        let status = if response.status
                                            == types::BinanceOrderStatus::Filled
                                        {
                                            BulkPostStatus::Filled {
                                                order_id: response.order_id.to_string(),
                                                size: response.executed_qty,
                                                average_price: response.avg_price,
                                            }
                                        } else {
                                            BulkPostStatus::Resting {
                                                order_id: response.order_id.to_string(),
                                            }
                                        };

                                        let result = BulkPostResult {
                                            instrument,
                                            order,
                                            status,
                                        };
                                        Some(Action::HandleExecutionResult(
                                            ExecutionResult::Orderbook(TimestampedData {
                                                data: OrderbookResults::BulkPost(vec![result]),
                                                emited_at: Timestamp::from(
                                                    Utc.timestamp_millis_opt(
                                                        response.update_time as i64,
                                                    )
                                                    .unwrap(),
                                                ),
                                                received_at: Timestamp::now(),
                                            }),
                                        ))
                                    }
                                    Err(e) => {
                                        error!("Failed to deserialize order response: {}", e);
                                        None
                                    }
                                }
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

    fn sign<T: serde::Serialize>(&self, payload: &T) -> Result<String> {
        let query = serde_urlencoded::to_string(payload)?;
        let signature = self.signer.sign(query.as_bytes());
        Ok(B64.encode(signature.to_bytes()))
    }
}

#[async_trait]
impl ExchangeExecution for BinancePerpExecutor {
    async fn post_orders(&self, orders: Vec<(Instrument, OrderInfo)>) -> Result<()> {
        for (instrument, order) in orders {
            let mut req = BinancePerpCreateOrderReq::from_order(
                instrument.clone(),
                order.clone(),
                self.api_key.clone(),
            );
            let signature: String = self.sign(&req)?;
            req.signature = Some(signature.into());

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
            let mut req = BinancePerpCancelOrderReq::from_cancel(
                instrument.clone(),
                order_ref.clone(),
                self.api_key.clone(),
            );
            let signature = self.sign(&req)?;
            req.signature = Some(signature.into());

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
    async fn test_binance_perp_cancel() {
        init_logging();

        let api_key = std::env::var("BINANCE_PERP_API_KEY").unwrap_or_default();
        let secret_key = std::env::var("BINANCE_PERP_SECRET_KEY").unwrap_or_default();
        let (changes_tx, _) = mpsc::channel(100);
        let executor = BinancePerpExecutor::new(api_key, secret_key, changes_tx).unwrap();

        let instrument = Instrument::from("BTCUSDT");
        let client_order_id = "test_order_12345";
        let order = OrderInfo {
            side: Side::Ask,
            price: dec!(95000),
            size: dec!(0.002),
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
