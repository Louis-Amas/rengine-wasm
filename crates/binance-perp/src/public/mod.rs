pub mod http;
pub mod types;
pub mod ws;

use crate::public::{types::BinancePublicMessage, ws::BinancePerpPublicStreamer};
use anyhow::Result;
use frunk_ws::{
    engine::{bind_stream, run_ws_loop},
    handler::to_handler,
    handlers::{
        forwarder::{forward_messages, ForwarderState, JsonParser},
        logging::{check_last_msg_timeout, update_last_msg, LastMsg},
    },
    types::{ConnectHandler, ContextState, HandlerOutcome},
};
use futures::{FutureExt, SinkExt};
use http::{BinancePerpPublicReader, HttpClient};
use rengine_non_wasm_types::{ChangesTx, TopBookRegistry};
use rengine_types::{Mapping, Venue};
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle, time};
use tokio_stream::wrappers::{IntervalStream, UnboundedReceiverStream};
use tokio_tungstenite::tungstenite::Message;
use tracing::error;

pub struct BinancePerpPublic {
    pub http: BinancePerpPublicReader,
    pub handles: Vec<JoinHandle<Result<()>>>,
}

impl BinancePerpPublic {
    pub async fn new(
        venue: Venue,
        changes_tx: ChangesTx,
        mapping: Mapping,
        registry: Arc<TopBookRegistry>,
    ) -> Self {
        let (incoming_msgs_tx, incoming_msgs_rx) =
            mpsc::unbounded_channel::<BinancePublicMessage>();

        let http = BinancePerpPublicReader {
            client: HttpClient::default(),
            venue: venue.clone(),
            changes_tx: changes_tx.clone(),
            mapping: mapping.clone(),
        };

        let venue_clone = venue.clone();

        let ws_handle = tokio::spawn(async move {
            let (_request_tx, request_rx) = mpsc::unbounded_channel::<String>();
            let request_stream = UnboundedReceiverStream::new(request_rx);

            type PerpConnectHandler = ConnectHandler<
                frunk::HList![ForwarderState<BinancePublicMessage>, LastMsg, ContextState],
            >;

            let on_connect: PerpConnectHandler = Box::new(move |ws, state| {
                let mapping = mapping.clone();
                let venue = venue_clone.clone();
                async move {
                    let last: &mut LastMsg = state.get_mut();
                    last.last_msg = chrono::Utc::now();

                    if let Some(instruments) = mapping.instruments(&venue) {
                        for details in instruments.values() {
                            let symbol =
                                format!("{}{}", details.base, details.quote).to_lowercase();

                            let book_stream = format!("{}@bookTicker", symbol);
                            let trade_stream = format!("{}@aggTrade", symbol);
                            let subscribe_msg = serde_json::json!({
                                "method": "SUBSCRIBE",
                                "params": [book_stream, trade_stream],
                                "id": 1
                            })
                            .to_string();

                            if let Err(e) = ws.send(Message::Text(subscribe_msg)).await {
                                error!("Failed to send subscription request: {}", e);
                            }
                        }
                    }
                }
                .boxed()
            });

            let forwarder_state = ForwarderState {
                sender: incoming_msgs_tx.clone(),
            };
            let context_state = ContextState::new("BinancePerpPublic");
            let state = frunk::hlist![forwarder_state, LastMsg::default(), context_state];

            static PARSER: JsonParser<BinancePublicMessage> = JsonParser::new();

            let handler = frunk::hlist![
                to_handler(move |ws, state, msg| {
                    forward_messages(ws, state, msg, &PARSER, |_| false)
                }),
                to_handler(update_last_msg)
            ];

            let request_stream = bind_stream(request_stream, |ws, _state, msg| {
                async move {
                    if let Err(e) = ws.send(Message::Text(msg)).await {
                        error!("Failed to send request: {}", e);
                    }
                    HandlerOutcome::Continue
                }
                .boxed()
            });

            let watchdog_stream = IntervalStream::new(time::interval(Duration::from_millis(500)));
            let action_watchdog = bind_stream(watchdog_stream, |ws, state, _| {
                check_last_msg_timeout(ws, state, Duration::from_secs(1))
            });

            if let Err(e) = run_ws_loop(
                "wss://fstream.binance.com/ws".to_string(),
                state,
                vec![on_connect],
                handler,
                vec![request_stream, action_watchdog],
            )
            .await
            {
                error!("WS loop failed: {}", e);
            }
            Ok(())
        });

        let streamer = BinancePerpPublicStreamer::new(venue, changes_tx, registry);
        let handle_connection_handle = tokio::spawn(streamer.handle_connection(incoming_msgs_rx));

        Self {
            http,
            handles: vec![ws_handle, handle_connection_handle],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_binance_perp_fetch_market_specs() {
        let http = HttpClient::default();
        let specs = http.all_market_specs().await.unwrap();

        println!("Binance Perpetual markets: {} total", specs.len());
        for (symbol, spec) in specs.iter().take(5) {
            println!("{symbol} {spec:?}");
        }

        assert!(!specs.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_binance_perp_fetch_funding_rates() {
        let http = HttpClient::default();
        let funding_rates = http.funding_rates().await.unwrap();

        println!(
            "Binance Perpetual funding rates: {} total",
            funding_rates.len()
        );
        for rate in funding_rates.iter().take(5) {
            println!(
                "{}: {} at {:?}",
                rate.symbol, rate.funding_rate, rate.funding_time
            );
        }

        assert!(!funding_rates.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_book_ticker_stream() {
        let venue: Venue = "binance-perp".into();
        let (changes_tx, _changes_rx) = tokio::sync::mpsc::channel(100);
        let mapping = Mapping::default();
        let (registry, mut register_rx) = TopBookRegistry::new();

        let exchange = BinancePerpPublic::new(venue, changes_tx, mapping, registry).await;

        // Wait for registration
        let (key, mut receiver) = tokio::time::timeout(Duration::from_secs(10), register_rx.recv())
            .await
            .expect("timeout waiting for registration")
            .expect("register_rx closed");

        println!("Registered key: {:?}", key);

        // Wait for update
        let _ = tokio::time::timeout(Duration::from_secs(10), receiver.changed())
            .await
            .expect("timeout waiting for update")
            .expect("receiver closed");

        let book = receiver.borrow().clone();
        println!("Received book: {:?}", book);

        // Clean up handles
        for handle in exchange.handles {
            handle.abort();
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_agg_perp_trade_stream() {
        use rengine_types::MappingInner;

        let venue: Venue = "binance-perp".into();
        let (changes_tx, mut changes_rx) = tokio::sync::mpsc::channel(100);

        let toml_str = r#"
[instrument_mapping."binance-perp".BTCUSDT]
base = "btc"
quote = "usdt"
marketType = "perp"

[token_mapping]
"#;
        let mapping_inner: MappingInner = toml::from_str(toml_str).unwrap();
        let mapping = Mapping::new(mapping_inner);

        let (registry, _register_rx) = TopBookRegistry::new();

        let exchange = BinancePerpPublic::new(venue, changes_tx, mapping, registry).await;

        // Wait for trade flow action
        let action = tokio::time::timeout(Duration::from_secs(10), changes_rx.recv())
            .await
            .expect("timeout waiting for trade flow")
            .expect("changes_rx closed");

        println!("Received action: {:?}", action);

        // Clean up handles
        for handle in exchange.handles {
            handle.abort();
        }
    }
}
