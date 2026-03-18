use std::net::SocketAddr;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod build;
mod cargo_gen;
mod check;
mod routes;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("wasm_builder=info".parse()?))
        .init();

    let deps_dir =
        std::env::var("WASM_BUILDER_DEPS_DIR").unwrap_or_else(|_| "/deps".to_string());
    let target_dir =
        std::env::var("WASM_BUILDER_TARGET_DIR").unwrap_or_else(|_| "/tmp/wasm-builder-target".to_string());

    // Ensure the shared target directory exists
    std::fs::create_dir_all(&target_dir)?;

    let cancel = CancellationToken::new();
    let state = routes::AppState::new(deps_dir, target_dir, cancel.clone());

    let app = routes::build_router(state);

    let addr: SocketAddr = std::env::var("WASM_BUILDER_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3001".to_string())
        .parse()?;

    info!("wasm-builder listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(cancel))
        .await?;

    info!("wasm-builder shutdown complete");
    Ok(())
}

async fn shutdown_signal(cancel: CancellationToken) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("shutdown signal received, cancelling in-flight builds...");
    cancel.cancel();
}
