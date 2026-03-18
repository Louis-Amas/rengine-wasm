pub(crate) mod http;
pub(crate) mod ws;

use crate::{
    hyperliquid::HyperLiquidPublicConfig,
    public::{http::HyperLiquidPublicReader, ws::HyperLiquidPublicStreamer},
    ws::{ExtraData, Subscription, WsClient, WsHyperliquidMessage},
};
use anyhow::Result;
use rengine_non_wasm_types::{ChangesTx, TopBookRegistry};
use rengine_types::{Mapping, Venue};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};
use tracing::info;

pub struct HyperLiquidPublic {
    pub http: HyperLiquidPublicReader,
    pub handles: Vec<JoinHandle<Result<()>>>,
}

impl HyperLiquidPublic {
    pub fn new(
        venue: Venue,
        config: HyperLiquidPublicConfig,
        changes_tx: ChangesTx,
        mapping: Mapping,
        registry: Arc<TopBookRegistry>,
    ) -> Self {
        let (_, outcoming_message_rx) =
            mpsc::unbounded_channel::<(WsHyperliquidMessage, Option<ExtraData>)>();

        let (incoming_msgs_tx, incoming_msgs_rx) = mpsc::unbounded_channel();

        info!("{venue} configured for markets {:?}", config.markets);

        let subs: Vec<_> = config
            .markets
            .clone()
            .into_iter()
            .flat_map(|market| {
                vec![
                    Subscription::L2Book {
                        coin: market.symbol.clone(),
                    },
                    Subscription::Trades {
                        coin: market.symbol,
                    },
                ]
            })
            .collect();

        let (reconnect_tx, reconnect_rx) = broadcast::channel::<()>(1);
        let ws_client = WsClient::new(
            subs,
            reconnect_tx,
            incoming_msgs_tx,
            outcoming_message_rx,
            Duration::from_secs(10),
        );
        let ws_run_handle = tokio::spawn(ws_client.run());

        let streamer_reader =
            HyperLiquidPublicStreamer::new(venue.clone(), changes_tx.clone(), registry);

        let handle_connection_handle =
            tokio::spawn(streamer_reader.handle_connection(incoming_msgs_rx));

        let reader = HyperLiquidPublicReader::new(
            venue.clone(),
            changes_tx.clone(),
            config.markets.clone(),
            mapping.clone(),
        );

        let run_on_reconnect_handle = tokio::spawn(reader.run_on_reconnect(reconnect_rx));

        let reader = HyperLiquidPublicReader::new(venue, changes_tx, config.markets, mapping);

        Self {
            http: reader,
            handles: vec![
                ws_run_handle,
                handle_connection_handle,
                run_on_reconnect_handle,
            ],
        }
    }

    pub fn stop(&self) {
        for handle in &self.handles {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rengine_interfaces::PublicExchangeReader;
    use rengine_types::Action;
    use tokio::sync::mpsc;

    #[tokio::test]
    #[ignore]
    async fn test_fetch_market_specs() {
        let (tx, mut rx) = mpsc::channel(100);

        let venue = "hyperliquid".into();
        let toml_str = r#"
[instrument_mapping]

[token_mapping."hyperliquid"]
"ETH" = "eth"
"BTC" = "btc"
"@151" = "eth-spot"
"#;
        let mapping_inner: rengine_types::MappingInner = toml::from_str(toml_str).unwrap();
        let mapping = Mapping::new(mapping_inner);
        let config = HyperLiquidPublicConfig::default();

        let reader = HyperLiquidPublicReader::new(venue, tx, config.markets, mapping);
        reader.fetch_market_specs().await.unwrap();

        // Read from channel
        println!("Reading market specs from channel...");
        let mut count = 0;
        while let Ok(actions) = rx.try_recv() {
            for action in actions {
                if let Action::SetMarketSpec(key, spec) = action {
                    println!("Received spec for {}: {:?}", key.instrument, spec);
                    count += 1;
                }
            }
        }
        println!("Total specs received: {}", count);
    }
}
