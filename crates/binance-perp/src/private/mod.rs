pub mod handlers;
pub mod http;
pub mod types;

use crate::{
    execution::BinancePerpExecutor,
    private::{
        handlers::{handle_account_update, handle_order_trade_update},
        http::BinancePerpPrivateReader,
        types::{
            BinancePrivateMessage, BinanceUserDataEvent, BinanceWsApiResponse, ListenKeyResponse,
        },
    },
};
use anyhow::{Context, Result};
use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{pkcs8::DecodePrivateKey as _, Signer, SigningKey};
use frunk_ws::{
    engine::{bind_stream, run_ws_loop},
    handler::to_handler,
    handlers::forwarder::{forward_messages, ForwarderState, JsonParser},
    types::{ConnectHandler, ContextState, HandlerOutcome},
};
use futures::{future::BoxFuture, stream, FutureExt, SinkExt};
use rengine_interfaces::ExchangePrivateReader;
use rengine_types::{identifiers::Account, state::Action, Mapping};
use std::{
    collections::BTreeMap,
    env,
    sync::{Arc, Mutex},
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time,
};
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream};
use tracing::error;

static PARSER: JsonParser<BinancePrivateMessage> = JsonParser::new();

type ChangesTx = mpsc::Sender<Vec<Action>>;
type State = frunk::HList![ForwarderState<BinancePrivateMessage>, ContextState];

struct KeyState {
    listen_key: Option<String>,
    api_key: String,
}

pub struct BinancePerpPrivate {
    pub exchange: BinancePerpExecutor,
    pub http: BinancePerpPrivateReader,
    pub handles: Vec<JoinHandle<Result<()>>>,
}

const WS_URL: &str = "wss://ws-fapi.binance.com/ws-fapi/v1";
const STREAM_URL: &str = "wss://fstream.binance.com/ws";

impl BinancePerpPrivate {
    pub async fn new(
        account: Account,
        changes_tx: ChangesTx,
        mapping: Mapping,
        api_key: String,
        secret_key: String,
    ) -> Result<Self> {
        let (incoming_msgs_tx, incoming_msgs_rx) =
            mpsc::unbounded_channel::<BinancePrivateMessage>();

        let http = BinancePerpPrivateReader::new(
            api_key.clone(),
            secret_key.clone(),
            account.clone(),
            changes_tx.clone(),
            mapping.clone(),
        )?;
        let exchange =
            BinancePerpExecutor::new(api_key.clone(), secret_key.clone(), changes_tx.clone())?;

        let (listen_key, key_handle) = get_listen_key(&api_key, &secret_key).await?;
        let stream_url = format!("{}/{}", STREAM_URL, listen_key);

        let reader = http.clone();
        let changes_tx_for_ws = changes_tx.clone();

        let ws_handle = tokio::spawn(async move {
            let url = stream_url;

            let forwarder_state = ForwarderState {
                sender: incoming_msgs_tx.clone(),
            };
            let context_state = ContextState::new("BinancePerpPrivate");
            let state = frunk::hlist![forwarder_state, context_state];

            let handler = to_handler(move |ws, state, msg| {
                forward_messages(ws, state, msg, &PARSER, |_| false)
            });

            let on_connect: ConnectHandler<State> = Box::new(move |_, _| {
                let reader = reader.clone();
                let changes_tx = changes_tx_for_ws.clone();
                async move {
                    if let Err(e) = reader.sync_state(&changes_tx).await {
                        error!("Failed to sync state: {}", e);
                    }
                }
                .boxed()
            });

            if let Err(e) = run_ws_loop(url, state, vec![on_connect], handler, vec![]).await {
                error!("WS loop failed: {}", e);
            }
            Ok(())
        });

        let handle_msgs = tokio::spawn(async move {
            let mut rx = incoming_msgs_rx;
            while let Some(msg) = rx.recv().await {
                match msg {
                    BinancePrivateMessage::UserData(event) => {
                        let actions = match event {
                            BinanceUserDataEvent::AccountUpdate {
                                event_time,
                                update_data,
                            } => handle_account_update(event_time, update_data, account.clone()),
                            BinanceUserDataEvent::OrderTradeUpdate { event_time, order } => {
                                handle_order_trade_update(
                                    event_time,
                                    *order,
                                    account.clone(),
                                    &mapping,
                                )
                            }
                            _ => vec![],
                        };

                        if !actions.is_empty() {
                            if let Err(e) = changes_tx.send(actions).await {
                                error!("Failed to send changes: {}", e);
                            }
                        }
                    }
                    BinancePrivateMessage::Unknown(val) => {
                        if let Some(obj) = val.as_object() {
                            if obj.contains_key("error") {
                                error!("Received error from Binance: {:?}", val);
                            } else if let Some(status) = obj.get("status").and_then(|s| s.as_i64())
                            {
                                if status != 200 {
                                    error!("Received bad status from Binance: {:?}", val);
                                }
                            }
                        }
                    }
                }
            }
            Ok(())
        });

        Ok(Self {
            exchange,
            http,
            handles: vec![ws_handle, handle_msgs, key_handle],
        })
    }
}

async fn send_logon(
    ws: &mut tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>,
    api_key: &str,
    secret_key: &str,
) -> Result<()> {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let mut params = BTreeMap::from_iter(vec![
        ("apiKey", api_key.to_string()),
        ("timestamp", timestamp.to_string()),
    ]);

    let signature = match signer_from_pem_b64(secret_key) {
        Ok(signer) => {
            let query_str = serde_urlencoded::to_string(&params).unwrap_or_default();
            B64.encode(signer.sign(query_str.as_bytes()).to_bytes())
        }
        Err(e) => return Err(anyhow::anyhow!("Failed to create signer: {}", e)),
    };

    params.insert("signature", signature);

    let logon_req = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "method": "session.logon",
        "params": params
    });

    ws.send(Message::Text(logon_req.to_string()))
        .await
        .map_err(|e| anyhow::anyhow!(e))
}

async fn send_subscribe(
    ws: &mut tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>,
    api_key: &str,
) -> Result<()> {
    let subscribe_req = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "method": "userDataStream.start",
        "params": {
            "apiKey": api_key
        }
    });

    ws.send(Message::Text(subscribe_req.to_string()))
        .await
        .map_err(|e| anyhow::anyhow!(e))
}

async fn get_listen_key(
    api_key: &str,
    secret_key: &str,
) -> Result<(String, JoinHandle<Result<()>>)> {
    let url = env::var("BINANCE_PERP_WS_API_URL").unwrap_or_else(|_| WS_URL.to_string());
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));

    let api_key = api_key.to_string();
    let secret_key = secret_key.to_string();

    let handle = tokio::spawn(async move {
        let state = frunk::hlist![
            KeyState {
                listen_key: None,
                api_key: api_key.clone()
            },
            ContextState::new("BinancePerpKey")
        ];

        let handler = to_handler(
            move |_ws, state: &mut frunk::HList![KeyState, ContextState], msg| {
                if let Message::Text(text) = msg {
                    if let Ok(resp) =
                        serde_json::from_str::<BinanceWsApiResponse<ListenKeyResponse>>(text)
                    {
                        state.get_mut::<KeyState, _>().listen_key =
                            Some(resp.result.listen_key.clone());
                        if let Some(tx) = tx.lock().unwrap().take() {
                            let _ = tx.send(resp.result.listen_key);
                        }
                    }
                }
                async { Ok(HandlerOutcome::Continue) }.boxed()
            },
        );

        let on_connect: ConnectHandler<frunk::HList![KeyState, ContextState]> =
            Box::new(move |ws, _state| {
                let api_key = api_key.clone();
                let secret_key = secret_key.clone();
                async move {
                    if let Err(e) = send_logon(ws, &api_key, &secret_key).await {
                        error!("Logon failed: {}", e);
                    }
                    if let Err(e) = send_subscribe(ws, &api_key).await {
                        error!("Subscribe failed: {}", e);
                    }
                }
                .boxed()
            });

        let ping_stream = stream::unfold((), |_| async {
            time::sleep(time::Duration::from_secs(1500)).await;
            Some(((), ()))
        });

        fn ping_logic_fn<'a>(
            ws: &'a mut tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>,
            state: &'a mut frunk::HList![KeyState, ContextState],
            _: (),
        ) -> BoxFuture<'a, HandlerOutcome> {
            let (listen_key, api_key) = {
                let k = state.get::<KeyState, _>();
                (k.listen_key.clone(), k.api_key.clone())
            };
            async move {
                if let Some(listen_key) = listen_key {
                    let req = serde_json::json!({
                        "id": uuid::Uuid::new_v4().to_string(),
                        "method": "userDataStream.ping",
                        "params": {
                            "apiKey": api_key,
                            "listenKey": listen_key
                        }
                    });
                    if let Err(e) = ws.send(Message::Text(req.to_string())).await {
                        error!("Failed to send ping: {}", e);
                    }
                }
                HandlerOutcome::Continue
            }
            .boxed()
        }

        let input_streams = vec![bind_stream(ping_stream, ping_logic_fn)];

        if let Err(e) = run_ws_loop(url, state, vec![on_connect], handler, input_streams).await {
            error!("Key loop failed: {}", e);
        }
        Ok(())
    });

    let listen_key = rx
        .await
        .map_err(|_| anyhow::anyhow!("Failed to receive listen key"))?;

    Ok((listen_key, handle))
}

pub(crate) fn signer_from_pem_b64(pem_b64: impl AsRef<str>) -> Result<SigningKey> {
    let pem_bytes = B64
        .decode(pem_b64.as_ref())
        .context("decoding base64 PEM content")?;
    let pem = String::from_utf8(pem_bytes).context("converting PEM bytes to UTF-8 string")?;
    SigningKey::from_pkcs8_pem(&pem).context("parsing Ed25519 PKCS#8 PEM")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rengine_types::MarketType;
    use rengine_utils::init_logging;

    #[tokio::test]
    #[ignore]
    async fn test_binance_perp_private_integration() {
        init_logging();
        // This test requires valid API credentials
        // Set environment variables: BINANCE_PERP_API_KEY and BINANCE_SECRET_KEY
        let api_key = env::var("BINANCE_PERP_API_KEY").expect("BINANCE_PERP_API_KEY not set");
        let secret_key = env::var("BINANCE_PERP_SECRET_KEY").expect("BINANCE_SECRET_KEY not set");

        let (changes_tx, mut changes_rx) = mpsc::channel(100);
        let account = Account {
            venue: "binance_perp".into(),
            market_type: MarketType::Perp,
            account_id: "test_account".into(),
        };

        let mapping = rengine_types::Mapping::default();

        let binance_private =
            BinancePerpPrivate::new(account.clone(), changes_tx, mapping, api_key, secret_key)
                .await
                .unwrap();

        println!("Binance Private initialized, listening for user data events...");

        // Wait for some events
        time::sleep(time::Duration::from_secs(10)).await;

        let mut count = 0;
        while let Ok(actions) = changes_rx.try_recv() {
            for action in actions {
                println!("Received action: {:?}", action);
                count += 1;
                if count >= 3 {
                    break;
                }
            }
            if count >= 3 {
                break;
            }
        }

        println!("Received {} actions", count);

        // Clean up handles
        for handle in binance_private.handles {
            handle.abort();
        }
    }
}
