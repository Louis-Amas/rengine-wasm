use crate::types::NewHead;
use anyhow::Result;
use frunk::{hlist, hlist::Selector, HCons, HNil};
use frunk_ws::{
    engine::{bind_stream, run_ws_loop},
    handlers::logging::{check_last_msg_timeout, update_last_msg, LastMsg},
    types::{ConnectHandler, ContextState, HandlerOutcome, WsStream},
};
use futures::{future::BoxFuture, FutureExt, SinkExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{error, info, warn};

/// evm_ws JSON-RPC request
#[derive(Serialize, Debug)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: serde_json::Value,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(bound(deserialize = "T: DeserializeOwned"))]
pub struct SubscriptionParams<T: DeserializeOwned> {
    // pub subscription: String,
    pub result: T,
}

/// evm_ws JSON-RPC subscription message
#[derive(Deserialize, Debug, Clone)]
#[serde(bound(deserialize = "T: DeserializeOwned"))]
#[serde(tag = "method", rename_all = "camelCase")]
pub enum EthWsMessage<T: DeserializeOwned> {
    #[serde(rename = "eth_subscription")]
    Subscription { params: SubscriptionParams<T> },
    #[serde(other)]
    Unknown,
}

// State types
#[derive(Clone)]
pub struct BlockSender(pub watch::Sender<NewHead>);

#[derive(Clone)]
pub struct ReconnectSender(pub broadcast::Sender<()>);

// Type alias for the state
pub type WsState =
    HCons<BlockSender, HCons<LastMsg, HCons<ReconnectSender, HCons<ContextState, HNil>>>>;

pub struct EvmWebsocketClient {
    reconnect_signal_tx: broadcast::Sender<()>,
    latest_block: watch::Sender<NewHead>,
    idle_timeout: Duration,
}

impl EvmWebsocketClient {
    pub(crate) fn new(
        reconnect_signal_tx: broadcast::Sender<()>,
        latest_block: watch::Sender<NewHead>,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            reconnect_signal_tx,
            latest_block,
            idle_timeout,
        }
    }

    pub(crate) async fn run(self, url: String) {
        let state: WsState = hlist![
            BlockSender(self.latest_block),
            LastMsg::default(),
            ReconnectSender(self.reconnect_signal_tx),
            ContextState::new("EvmWebsocketClient")
        ];

        let on_connect: Vec<ConnectHandler<WsState>> =
            vec![Box::new(|ws: &mut WsStream, state: &mut WsState| {
                async move {
                    // Send reconnect signal
                    let sender: &mut ReconnectSender = state.get_mut();
                    let _ = sender.0.send(());

                    // Subscribe
                    let subscribe_msg = serde_json::to_string(&JsonRpcRequest {
                        jsonrpc: "2.0",
                        id: 1,
                        method: "eth_subscribe",
                        params: serde_json::json!(["newHeads"]),
                    })
                    .unwrap();

                    if let Err(err) = ws.send(WsMessage::Text(subscribe_msg)).await {
                        error!("[evm_ws] Failed to send subscription: {err:?}");
                    } else {
                        info!("[evm_ws] Subscribed to newHeads");
                    }
                }
                .boxed()
            })];

        // Handlers
        let handlers = hlist![update_last_msg, handle_subscription];

        // Streams
        let ping_stream = bind_stream(
            tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
                Duration::from_secs(30),
            )),
            |ws, _state: &mut WsState, _| {
                async move {
                    if let Err(err) = ws.send(WsMessage::Ping(vec![])).await {
                        warn!("[evm_ws] Ping failed: {err:?}");
                    }
                    HandlerOutcome::Continue
                }
                .boxed()
            },
        );

        let idle_timeout = self.idle_timeout;
        let watchdog_stream = bind_stream(
            tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
                Duration::from_secs(1),
            )),
            move |ws, state: &mut WsState, _| check_last_msg_timeout(ws, state, idle_timeout),
        );

        if let Err(e) = run_ws_loop(
            url,
            state,
            on_connect,
            handlers,
            vec![ping_stream, watchdog_stream],
        )
        .await
        {
            error!("[evm_ws] Loop failed: {e:?}");
        }
    }
}

// Handler implementations
fn handle_subscription<'a, S, I>(
    _ws: &'a mut WsStream,
    state: &'a mut S,
    msg: &'a WsMessage,
) -> BoxFuture<'a, Result<HandlerOutcome>>
where
    S: Selector<BlockSender, I> + Send + 'static,
{
    async move {
        if let WsMessage::Text(raw) = msg {
            let parsed = serde_json::from_str::<EthWsMessage<NewHead>>(raw);
            match parsed {
                Ok(EthWsMessage::Subscription { params }) => {
                    let sender: &mut BlockSender = state.get_mut();
                    if let Err(err) = sender.0.send(params.result) {
                        error!("[evm_ws] Failed to send message to channel: {err:?}");
                    }
                }
                Err(err) => {
                    if !raw.contains("\"result\"") {
                        warn!("[evm_ws] Could not parse msg: {raw}, err: {err:?}");
                    }
                }
                _ => {}
            }
        }
        Ok(HandlerOutcome::Continue)
    }
    .boxed()
}

#[cfg(test)]
mod test {
    use crate::{types::NewHead, ws::EvmWebsocketClient};
    use alloy::{
        node_bindings::Anvil,
        primitives::B256,
        providers::{ext::AnvilApi, ProviderBuilder},
    };
    use rengine_types::Timestamp;
    use std::time::Duration;
    use tokio::sync::{broadcast, watch};

    #[tokio::test]
    async fn evm_ws_streams_new_heads_from_anvil() {
        // 1) Spawn a local Anvil with WS enabled (default) and deterministic mining
        //    Anvil auto-mines on each tx; we’ll force blocks using `evm_mine`.
        let anvil = Anvil::new().spawn();

        let ws_url = anvil.ws_endpoint();
        let http_url = anvil.endpoint();

        // 2) Client wiring: broadcast (reconnect signal) + watch (latest block)
        let (reconnect_tx, _reconnect_rx) = broadcast::channel::<()>(8);

        let init_head = NewHead {
            number: 0,
            timestamp: Timestamp::now(),
            hash: B256::ZERO,
        };
        let (latest_block_tx, mut latest_block_rx) = watch::channel(init_head);

        // 3) Start your WS client in the background with a short idle timeout
        let client = EvmWebsocketClient::new(
            reconnect_tx.clone(),
            latest_block_tx,
            Duration::from_secs(15),
        );

        let ws_task = tokio::spawn(async move {
            // Run until the test ends; we ignore the result to avoid panicking
            let _ = client.run(ws_url).await;
        });

        // 4) Give the subscription a moment to establish
        tokio::time::sleep(Duration::from_millis(300)).await;

        // 5) Use HTTP provider to mine a few blocks, triggering `newHeads` over WS
        let http = ProviderBuilder::new().connect_http(http_url.parse().unwrap());
        http.anvil_mine(Some(3), None).await.unwrap();

        // 6) Wait (with timeout) until we observe a head > 0 via the watch channel
        let observed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                latest_block_rx.changed().await.unwrap();
                let head = latest_block_rx.borrow();
                if head.number > 0 {
                    break head.clone();
                }
            }
        })
        .await
        .expect("timed out waiting for newHeads");

        // Basic sanity checks on the observed head
        assert!(observed.number >= 1, "expected mined block number >= 1");
        assert_ne!(observed.hash, B256::ZERO, "expected a non-zero block hash");

        // 7) Clean up
        // Drop Anvil (on scope end) and stop the task
        // (The task loops forever; we just detach and let the test end.)
        ws_task.abort();
    }
}
