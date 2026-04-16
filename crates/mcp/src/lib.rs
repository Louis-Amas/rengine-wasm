mod tools;

use anyhow::Result;
use parking_lot::RwLock;
use rengine_core::{EvmReaders, StrategiesHandler, TransformersHandler};
use rengine_types::{ExecutionRequest, State};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::{net::TcpListener, sync::mpsc::UnboundedSender, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::info;

#[derive(Clone)]
pub struct McpState {
    pub(crate) strategies_handler: StrategiesHandler,
    #[allow(dead_code)]
    pub(crate) transformers_handler: TransformersHandler,
    pub(crate) evm_readers: EvmReaders,
    pub(crate) state: Arc<RwLock<State>>,
    pub(crate) external_requests_tx: UnboundedSender<ExecutionRequest>,
    pub(crate) wasm_builder_url: Option<String>,
    pub(crate) http_client: reqwest::Client,
}

impl McpState {
    pub fn new(
        strategies_handler: StrategiesHandler,
        transformers_handler: TransformersHandler,
        evm_readers: EvmReaders,
        state: Arc<RwLock<State>>,
        external_requests_tx: UnboundedSender<ExecutionRequest>,
        wasm_builder_url: Option<String>,
    ) -> Self {
        Self {
            strategies_handler,
            transformers_handler,
            evm_readers,
            state,
            external_requests_tx,
            wasm_builder_url,
            http_client: reqwest::Client::new(),
        }
    }
}

pub async fn spawn_mcp_server(
    state: McpState,
    addr: SocketAddr,
    cancellation_token: CancellationToken,
) -> Result<JoinHandle<Result<()>>> {
    info!("starting MCP server on {addr}");

    let config =
        StreamableHttpServerConfig::default().with_cancellation_token(cancellation_token.clone());

    let service = StreamableHttpService::new(
        move || Ok(state.clone()),
        LocalSessionManager::default().into(),
        config,
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = TcpListener::bind(addr).await?;

    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                cancellation_token.cancelled().await;
            })
            .await
            .map_err(|e| anyhow::anyhow!("MCP server error: {e}"))
    });

    Ok(handle)
}
