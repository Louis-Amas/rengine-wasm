pub(crate) mod http;
pub(crate) mod ws;

use crate::{
    hyperliquid::HyperLiquidPrivateConfig,
    private::{http::HyperLiquidPrivateReader, ws::HyperLiquidPrivateStreamer},
    ws::{ExtraData, Subscription, WsClient, WsHyperliquidMessage},
    HyperLiquidPerp,
};
use anyhow::Result;
use rengine_non_wasm_types::ChangesTx;
use rengine_types::{Account, Mapping};
use std::time::Duration;
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};
use tracing::info;

pub struct HyperLiquidPrivate {
    pub http: HyperLiquidPrivateReader,
    pub exchange: HyperLiquidPerp,
    pub handles: Vec<JoinHandle<Result<()>>>,
}

impl HyperLiquidPrivate {
    pub async fn new(
        account: Account,
        config: HyperLiquidPrivateConfig,
        mappings: Mapping,
        changes_tx: ChangesTx,
    ) -> Self {
        let trading_address = config.trading_account.signer.address();
        info!("new hyperliquid {account} with trading address {trading_address}");

        let (outcoming_message_tx, outcoming_message_rx) =
            mpsc::unbounded_channel::<(WsHyperliquidMessage, Option<ExtraData>)>();

        let (incoming_msgs_tx, incoming_msgs_rx) = mpsc::unbounded_channel();

        let subs = vec![
            Subscription::UserEvents {
                user: trading_address,
            },
            Subscription::OrderUpdates {
                user: trading_address,
            },
        ];

        let (reconnect_tx, reconnect_rx) = broadcast::channel::<()>(1);
        let ws_client = WsClient::new(
            subs,
            reconnect_tx,
            incoming_msgs_tx,
            outcoming_message_rx,
            Duration::from_secs(60 * 60 * 10),
        );
        let ws_run_handle = tokio::spawn(ws_client.run());

        let streamer =
            HyperLiquidPrivateStreamer::new(account.clone(), mappings.clone(), changes_tx.clone());

        let handle_connection_handle = tokio::spawn(streamer.handle_connection(incoming_msgs_rx));

        let reader = HyperLiquidPrivateReader::new(
            account.clone(),
            changes_tx.clone(),
            config.account_address,
            mappings.clone(),
        );
        let run_on_reconnect_handle = tokio::spawn(reader.run_on_reconnect(reconnect_rx));

        let reader = HyperLiquidPrivateReader::new(
            account,
            changes_tx,
            config.account_address,
            mappings.clone(),
        );

        let exchange = HyperLiquidPerp::new(
            config.trading_account.signer,
            mappings,
            outcoming_message_tx,
            config.max_response_duration,
        );

        Self {
            http: reader,
            exchange,
            handles: vec![
                ws_run_handle,
                run_on_reconnect_handle,
                handle_connection_handle,
            ],
        }
    }

    pub fn stop(&self) {
        for handle in &self.handles {
            handle.abort();
        }
    }
}
