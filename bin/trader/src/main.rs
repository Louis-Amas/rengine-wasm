use anyhow::Result;
use rengine_api::{spawn_http_api, AppState};
use rengine_config::Config;
use rengine_core::Engine;
use rengine_mcp::{spawn_mcp_server, McpState};
use std::env;
use tokio::{signal, sync::oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let filter_layer = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,wasmtime=warn,cranelift=warn,cranelift_codegen=warn")
    });

    // Enable file, line number, and include the instrumented "function" field
    let fmt_layer = fmt::Layer::default()
        .with_file(true) // show source file
        .with_line_number(true) // show source line
        .with_target(false); // disable module path if you like

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    info!("starting up Rengine :) !");

    let config_path =
        env::var("RENGINE_CONFIG").unwrap_or_else(|_| "config/config.toml".to_string());
    let config = Config::config_from_file(&config_path)?;
    let http_addr = format!("0.0.0.0:{}", config.http_api_port);

    let mcp_port = config.mcp_port;
    let (mut engine, external_requests_tx) = Engine::new(config).await?;

    let mcp_requests_tx = external_requests_tx.clone();
    let http_api_state = AppState::new(
        engine.strategies_handler.clone(),
        engine.transformers_handler.clone(),
        engine.evm_readers.clone(),
        engine.state(),
        external_requests_tx,
    );

    let http_handle = spawn_http_api(http_api_state, http_addr.parse()?).await?;

    let mcp_ct = CancellationToken::new();
    let mcp_handle = if let Some(mcp_port) = mcp_port {
        let mcp_state = McpState::new(
            engine.strategies_handler.clone(),
            engine.transformers_handler.clone(),
            engine.evm_readers.clone(),
            engine.state(),
            mcp_requests_tx,
        );
        let mcp_addr = format!("0.0.0.0:{mcp_port}");
        Some(spawn_mcp_server(mcp_state, mcp_addr.parse()?, mcp_ct.clone()).await?)
    } else {
        None
    };

    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    // Spawn signal handling task
    tokio::spawn(async move {
        let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())
            .expect("Failed to register SIGINT handler");
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to register SIGTERM handler");
        let mut sighup = signal::unix::signal(signal::unix::SignalKind::hangup())
            .expect("Failed to register SIGHUP handler");

        // Note: SIGSTOP cannot be caught or handled, so we don't include it

        tokio::select! {
            _ = sigint.recv() => {
                info!("Received SIGINT (Ctrl+C), initiating shutdown...");
                shutdown_tx.send(()).unwrap();
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating shutdown...");
                shutdown_tx.send(()).unwrap();
            }
            _ = sighup.recv() => {
                info!("Received SIGHUP, initiating shutdown...");
                shutdown_tx.send(()).unwrap();
            }
        }
    });

    // Run the engine with shutdown flag
    if let Err(err) = engine.crank(shutdown_rx).await {
        error!(?err);
    }

    http_handle.abort();
    mcp_ct.cancel();
    if let Some(h) = mcp_handle {
        h.abort();
    }

    info!("Application shutdown complete");
    Ok(())
}
