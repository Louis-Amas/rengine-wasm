pub mod http;
pub mod types;

use crate::{
    execution::{
        types::{BinanceOrderStatus, BinanceSide, BinanceTimeInForce},
        BinanceSpotExecutor,
    },
    private::{
        http::BinanceSpotPrivateReader,
        types::{BinanceSpotPrivateMessage, BinanceSpotPrivateMessageWrapper},
    },
    public::http::HttpClient,
};
use anyhow::{Context, Result};
use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
use chrono::TimeZone;
use ed25519_dalek::{ed25519::signature::SignerMut, pkcs8::DecodePrivateKey as _, SigningKey};
use frunk_ws::{
    engine::run_ws_loop,
    handler::{to_handler, WsHandler},
    handlers::forwarder::{forward_messages, ForwarderState, JsonParser},
    types::{Action as WsAction, ConnectHandler, ContextState},
};
use futures::{stream::BoxStream, FutureExt, SinkExt};
use rengine_interfaces::ExchangePrivateReader;
use rengine_non_wasm_types::ChangesTx;
use rengine_types::{
    Account, Action, BookKey, Instrument, Mapping, OpenOrder, OrderInfo, Side, Symbol, TimeInForce,
};
use std::{collections::BTreeMap, env};
use tokio::{net::TcpStream, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream};
use tracing::error;

type State = frunk::HList![
    ForwarderState<BinanceSpotPrivateMessageWrapper>,
    ContextState
];

pub struct BinanceSpotPrivate {
    pub exchange: BinanceSpotExecutor,
    pub http: HttpClient,
    pub handles: Vec<JoinHandle<Result<()>>>,
}

const WS_URL: &str = "wss://ws-api.binance.com:443/ws-api/v3";

impl BinanceSpotPrivate {
    pub async fn new(
        account: Account,
        changes_tx: ChangesTx,
        mapping: Mapping,
        api_key: String,
        secret_key: String,
    ) -> Result<Self> {
        let (incoming_msgs_tx, incoming_msgs_rx) =
            mpsc::unbounded_channel::<BinanceSpotPrivateMessageWrapper>();
        let http = HttpClient::default();
        let exchange =
            BinanceSpotExecutor::new(api_key.clone(), secret_key.clone(), changes_tx.clone())?;

        let reader = BinanceSpotPrivateReader::new(
            account.clone(),
            api_key.clone(),
            secret_key.clone(),
            changes_tx.clone(),
            mapping.clone(),
        )?;

        let url = env::var("BINANCE_SPOT_WS_API_URL").unwrap_or_else(|_| WS_URL.to_string());

        let changes_tx_for_ws = changes_tx.clone();
        let ws_handle = tokio::spawn(async move {
            let url = url;

            let forwarder_state = ForwarderState {
                sender: incoming_msgs_tx.clone(),
            };
            let context_state = ContextState::new("BinanceSpotPrivate");
            let state = frunk::hlist![forwarder_state, context_state];

            let handler = create_ws_handler();

            let on_connect = create_on_connect(api_key, secret_key, reader, changes_tx_for_ws);

            if let Err(e) = run_ws_loop(
                url,
                state,
                vec![on_connect],
                handler,
                Vec::<BoxStream<'static, WsAction<_>>>::new(),
            )
            .await
            {
                error!("WS loop failed: {}", e);
            }
            Ok(())
        });

        let handle_msgs = tokio::spawn(process_messages(
            incoming_msgs_rx,
            changes_tx, // Moved
            account,
            mapping,
        ));

        Ok(Self {
            exchange,
            http,
            handles: vec![ws_handle, handle_msgs],
        })
    }
}

fn create_ws_handler() -> impl WsHandler<State> {
    static PARSER: JsonParser<BinanceSpotPrivateMessageWrapper> = JsonParser::new();

    to_handler(move |ws, state, msg| {
        let ignore_fn = |msg: &Message| {
            if let Message::Text(text) = msg {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                    return v
                        .get("status")
                        .and_then(|s| s.as_u64())
                        .map(|s| s == 200)
                        .unwrap_or(false);
                }
            }
            false
        };
        forward_messages(ws, state, msg, &PARSER, ignore_fn)
    })
}

fn create_on_connect(
    api_key: String,
    secret_key: String,
    reader: BinanceSpotPrivateReader,
    changes_tx: ChangesTx,
) -> ConnectHandler<State> {
    Box::new(move |ws, _state| {
        let api_key = api_key.clone();
        let secret_key = secret_key.clone();
        let reader = reader.clone();
        let changes_tx = changes_tx.clone();
        async move {
            // 1. session.logon
            if let Err(e) = send_logon(ws, &api_key, &secret_key).await {
                error!("Failed to send session.logon: {}", e);
                return;
            }

            // 2. userDataStream.subscribe
            if let Err(e) = send_subscribe(ws).await {
                error!("Failed to send userDataStream.subscribe: {}", e);
            }

            // 3. Sync state
            if let Err(e) = reader.sync_state(&changes_tx).await {
                error!("Failed to sync state: {}", e);
            }
        }
        .boxed()
    })
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
        Ok(mut signer) => {
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
) -> Result<()> {
    let subscribe_req = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "method": "userDataStream.subscribe",
        "params": {}
    });

    ws.send(Message::Text(subscribe_req.to_string()))
        .await
        .map_err(|e| anyhow::anyhow!(e))
}

async fn process_messages(
    mut rx: mpsc::UnboundedReceiver<BinanceSpotPrivateMessageWrapper>,
    changes_tx: ChangesTx,
    account: Account,
    mapping: Mapping,
) -> Result<()> {
    while let Some(msg) = rx.recv().await {
        let mut actions = Vec::new();

        match msg.event {
            BinanceSpotPrivateMessage::OrderUpdate(report) => {
                let instrument = Instrument::from(report.symbol.as_str());
                let book_key = BookKey {
                    account: account.clone(),
                    instrument: instrument.clone(),
                };
                let order_id = report.order_id.to_string();

                match report.current_order_status {
                    BinanceOrderStatus::New => {
                        let side = match report.side {
                            BinanceSide::Buy => Side::Bid,
                            BinanceSide::Sell => Side::Ask,
                        };
                        let tif = match report.time_in_force {
                            BinanceTimeInForce::Gtc => TimeInForce::GoodUntilCancelled,
                            BinanceTimeInForce::Ioc | BinanceTimeInForce::Fok => {
                                TimeInForce::Unknown
                            }
                            BinanceTimeInForce::Gtx => TimeInForce::PostOnly,
                        };

                        let order_info = OrderInfo::new(side, report.price, report.quantity, tif)
                            .with_client_order_id(report.client_order_id.clone().into());

                        let open_order = OpenOrder {
                            info: order_info,
                            original_size: report.quantity,
                            is_snapshot: false,
                        };

                        actions.push(Action::SetOpenOrder(book_key.clone(), order_id, open_order));
                    }
                    BinanceOrderStatus::Canceled
                    | BinanceOrderStatus::Filled
                    | BinanceOrderStatus::Expired
                    | BinanceOrderStatus::Rejected => {
                        actions.push(Action::RemoveOpenOrder(book_key.clone(), order_id));
                    }
                    BinanceOrderStatus::PartiallyFilled => {
                        actions.push(Action::UpdateOpenOrder(
                            book_key.clone(),
                            order_id,
                            report.quantity - report.cumulative_filled_quantity,
                        ));
                    }
                }

                if matches!(
                    report.current_order_status,
                    BinanceOrderStatus::Filled | BinanceOrderStatus::PartiallyFilled
                ) {
                    if let Ok(details) = mapping.map_instrument(&account.venue, &instrument) {
                        let emitted_at = chrono::Utc
                            .timestamp_millis_opt(report.event_time as i64)
                            .single()
                            .map(rengine_types::Timestamp::from)
                            .unwrap_or_else(rengine_types::Timestamp::now);

                        let trade = rengine_types::Trade {
                            emitted_at,
                            received_at: rengine_types::Timestamp::now(),
                            order_id: report.order_id as i64,
                            trade_id: report.trade_id,
                            account: account.clone(),
                            base: details.base.clone(),
                            quote: details.quote.clone(),
                            side: match report.side {
                                BinanceSide::Buy => Side::Bid,
                                BinanceSide::Sell => Side::Ask,
                            },
                            market_type: details.market_type,
                            price: report.last_executed_price,
                            size: report.last_executed_quantity,
                            fee: report.commission_amount.unwrap_or_default(),
                            fee_symbol: report.commission_asset.clone().unwrap_or_default().into(),
                        };
                        actions.push(Action::RecordTrades(vec![(book_key, trade)]));
                    } else {
                        error!("Failed to map instrument {}", instrument);
                    }
                }
            }
            BinanceSpotPrivateMessage::OutboundAccountPosition(pos) => {
                for balance in pos.balances {
                    let asset_symbol = Symbol::from(balance.asset.as_str());
                    let symbol = match mapping.map_symbol(&account.venue, &asset_symbol) {
                        Ok(symbol) => symbol.clone(),
                        Err(err) => {
                            tracing::error!(?err, asset = %balance.asset, "Failed to map symbol");
                            continue;
                        }
                    };

                    let balance_key = rengine_types::BalanceKey {
                        account: account.clone(),
                        symbol,
                    };
                    // Assuming 'free' is the available balance.
                    // Rengine usually tracks total equity or available balance depending on strategy needs.
                    // Let's use free for now.
                    actions.push(Action::SetBalance(balance_key, balance.free));
                }
            }
            BinanceSpotPrivateMessage::BalanceUpdate(bal) => {
                let _balance_key = rengine_types::BalanceKey {
                    account: account.clone(),
                    symbol: bal.asset.clone().into(),
                };

                tracing::warn!("ignore BalanceUpdate: {:?}", bal);
            }
            BinanceSpotPrivateMessage::Unknown(_) => {}
        }

        if !actions.is_empty() {
            if let Err(e) = changes_tx.send(actions).await {
                error!("Failed to send actions: {}", e);
            }
        }
    }
    Ok(())
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
    use rengine_types::MappingInner;
    use std::env;

    #[tokio::test]
    #[ignore]
    async fn test_binance_spot_private_ws() {
        // Setup fake account and mapping
        let account = Account {
            account_id: "test".into(),
            venue: "binance-spot".into(),
            market_type: rengine_types::MarketType::Spot,
        };

        let toml_str = r#"
[instrument_mapping."binance-spot".BTCUSDC]
base = "btc"
quote = "usdc"
marketType = "spot"

[token_mapping]
"#;
        let mapping_inner: MappingInner = toml::from_str(toml_str).unwrap();
        let mapping = Mapping::new(mapping_inner);

        let (changes_tx, mut changes_rx) = mpsc::channel(100);

        // Needs valid API Key for real test, but this is ignored.
        let api_key = env::var("BINANCE_API_KEY").unwrap_or_default();
        let secret_key = env::var("BINANCE_SECRET_KEY").unwrap_or_default();

        if api_key.is_empty() || secret_key.is_empty() {
            println!("Skipping test due to missing API keys");
            return;
        }

        let _ws = BinanceSpotPrivate::new(account, changes_tx, mapping, api_key, secret_key)
            .await
            .unwrap();

        // Read from channel
        while let Some(actions) = changes_rx.recv().await {
            println!("Received actions: {:?}", actions);
        }
    }
}
