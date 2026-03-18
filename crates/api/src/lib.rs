use anyhow::{anyhow, Result};
use axum::{
    extract::{DefaultBodyLimit, Path, State as AxumState},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{delete, get, post, put},
    Json, Router,
};
use parking_lot::RwLock;
use rengine_core::{EvmReaders, StrategiesHandler, TransformersHandler};
use rengine_types::{ExecutionRequest, OrderActions, State, StrategyId, TransformerId, Venue};
use rengine_utils::as_base64;
use serde::Deserialize;
use std::{net::SocketAddr, sync::Arc};
use tokio::{net::TcpListener, sync::mpsc::UnboundedSender, task::JoinHandle};
use tower_http::cors::CorsLayer;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub strategies_handler: StrategiesHandler,
    pub transformers_handler: TransformersHandler,
    pub evm_readers: EvmReaders,
    pub state: Arc<RwLock<State>>,
    pub external_requests_tx: UnboundedSender<ExecutionRequest>,
}

impl AppState {
    pub const fn new(
        strategies_handler: StrategiesHandler,
        transformers_handler: TransformersHandler,
        evm_readers: EvmReaders,
        state: Arc<RwLock<State>>,
        external_requests_tx: UnboundedSender<ExecutionRequest>,
    ) -> Self {
        Self {
            strategies_handler,
            transformers_handler,
            evm_readers,
            state,
            external_requests_tx,
        }
    }
}

#[derive(Deserialize)]
struct TogglePayload {
    enabled: bool,
}

#[derive(Deserialize)]
struct WasmPayload {
    #[serde(with = "as_base64")]
    wasm: Vec<u8>,
}

/// POST /strategies/:id
/// Body: { wasm }
async fn add_strategy(
    AxumState(state): AxumState<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<WasmPayload>,
) -> impl IntoResponse {
    let id: StrategyId = id.into();
    info!("add strategy {id}");

    let result = state.strategies_handler.add(id, &payload.wasm, true).await;

    match result {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// POST /transformers/:id
/// Body: { wasm }
async fn add_transformer(
    AxumState(state): AxumState<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<WasmPayload>,
) -> impl IntoResponse {
    let id: TransformerId = id.into();
    info!("add transformer {id}");

    let result = state
        .transformers_handler
        .add(id, &payload.wasm, true)
        .await;

    match result {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// POST /evm/:id/multicall/:name
/// Body: { wasm }
async fn add_multicall(
    AxumState(state): AxumState<AppState>,
    Path((venue, id)): Path<(String, String)>,
    Json(payload): Json<WasmPayload>,
) -> impl IntoResponse {
    let venue: Venue = venue.into();
    info!(%venue, %id, "add multicall");

    let Some(reader) = state.evm_readers.get(&venue) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "evm reader not found" })),
        );
    };

    let result = reader.add_multicall_reader(id.into(), payload.wasm).await;

    match result {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// POST /evm/:id/multicall/:name
async fn remove_multicall(
    AxumState(state): AxumState<AppState>,
    Path((venue, id)): Path<(String, String)>,
) -> impl IntoResponse {
    let venue: Venue = venue.into();
    info!(%venue, %id, "add multicall");

    let Some(reader) = state.evm_readers.get(&venue) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "evm reader not found" })),
        );
    };

    let result = reader.remove_multicall_reader(id.into()).await;

    match result {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// POST /strategies/execute
/// Body: { wasm }
async fn execute_strategy(
    AxumState(state): AxumState<AppState>,
    Json(payload): Json<WasmPayload>,
) -> impl IntoResponse {
    info!("received execute strategy");
    match state
        .strategies_handler
        .instantiate_and_execute(&payload.wasm)
        .await
    {
        Ok(res) => {
            let json = serde_json::to_value(res).unwrap();
            (StatusCode::OK, Json(json))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

/// POST /transformers/execute
/// Body: { wasm }
async fn execute_transformer(
    AxumState(state): AxumState<AppState>,
    Json(payload): Json<WasmPayload>,
) -> impl IntoResponse {
    info!("received execute transformer");
    match state
        .transformers_handler
        .instantiate_and_execute(&payload.wasm)
        .await
    {
        Ok(res) => {
            let json = serde_json::to_value(res).unwrap();
            (StatusCode::OK, Json(json))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

/// PUT /strategies/:id/enabled
/// Body: { enabled: bool }
async fn toggle_strategy_enabled(
    AxumState(state): AxumState<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TogglePayload>,
) -> StatusCode {
    let id: StrategyId = id.into();
    info!("toggle strategy {id} {}", body.enabled);
    match state.strategies_handler.set_enabled(id, body.enabled).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::NOT_FOUND,
    }
}

/// PUT /transformers/:id/enabled
/// Body: { enabled: bool }
async fn toggle_transformer_enabled(
    AxumState(state): AxumState<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TogglePayload>,
) -> StatusCode {
    let id: TransformerId = id.into();
    info!("toggle transformer {id} {}", body.enabled);
    match state
        .transformers_handler
        .set_enabled(id, body.enabled)
        .await
    {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::NOT_FOUND,
    }
}

/// POST /evm/:venue/logs/:id
/// Body: { wasm }
async fn add_evm_logs(
    AxumState(state): AxumState<AppState>,
    Path((venue, id)): Path<(String, String)>,
    Json(payload): Json<WasmPayload>,
) -> impl IntoResponse {
    let venue: Venue = venue.into();
    info!(%venue, %id, "add evm logs");

    let Some(reader) = state.evm_readers.get(&venue) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "evm reader not found" })),
        );
    };

    let result = reader.add_log_reader(id, payload.wasm).await;

    match result {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// DELETE /evm/:venue/logs/:id
async fn remove_evm_logs(
    AxumState(state): AxumState<AppState>,
    Path((venue, id)): Path<(String, String)>,
) -> impl IntoResponse {
    let venue: Venue = venue.into();
    info!(%venue, %id, "remove evm logs");

    let Some(reader) = state.evm_readers.get(&venue) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "evm reader not found" })),
        );
    };

    let result = reader.remove_log_reader(id).await;

    match result {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// GET /state
async fn get_state(AxumState(state): AxumState<AppState>) -> impl IntoResponse {
    let state = state.state.read().clone();
    Json(state)
}

/// POST /orders
async fn post_orders(
    AxumState(state): AxumState<AppState>,
    Json(action): Json<OrderActions>,
) -> impl IntoResponse {
    let request = ExecutionRequest::Orderbook(action);
    match state.external_requests_tx.send(request) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// DELETE /orders
async fn delete_orders(
    AxumState(state): AxumState<AppState>,
    Json(action): Json<OrderActions>,
) -> impl IntoResponse {
    let request = ExecutionRequest::Orderbook(action);
    match state.external_requests_tx.send(request) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn index_handler() -> impl IntoResponse {
    Html(include_str!("../static/index.html"))
}

/// Build the router so you can test it directly if needed.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/strategies/execute", post(execute_strategy))
        .route("/strategies/{id}", post(add_strategy))
        .route("/strategies/{id}/toggle", put(toggle_strategy_enabled))
        .route("/transformers/execute", post(execute_transformer))
        .route("/transformers/{id}", post(add_transformer))
        .route("/transformers/{id}/toggle", put(toggle_transformer_enabled))
        .route("/evm/{venue}/multicall/{id}", post(add_multicall))
        .route("/evm/{venue}/multicall/{id}", delete(remove_multicall))
        .route("/evm/{venue}/logs/{id}", post(add_evm_logs))
        .route("/evm/{venue}/logs/{id}", delete(remove_evm_logs))
        .route("/state", get(get_state))
        .route("/orders", post(post_orders).delete(delete_orders))
        .route("/app", get(index_handler))
        .layer(CorsLayer::permissive())
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024))
        .with_state(state)
}

pub async fn spawn_http_api(state: AppState, addr: SocketAddr) -> Result<JoinHandle<Result<()>>> {
    info!("starting http api {addr:?}");
    let listener = TcpListener::bind(addr).await?;
    let app = build_router(state);

    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .map_err(|e| anyhow!("server error: {e}"))
    });

    Ok(handle)
}

#[cfg(test)]
mod tests {

    use rengine_types::{
        Account, ExecutionType, Instrument, MarketType, OrderActions, OrderInfo, Side, TimeInForce,
    };
    use rust_decimal::Decimal;
    use std::str::FromStr;

    #[test]
    fn test_order_actions_serialization() {
        let account = Account {
            venue: "hyperliquid".into(),
            market_type: MarketType::Spot,
            account_id: "hotwallet".into(),
        };

        let instrument: Instrument = "btc".into();

        let order_info = OrderInfo {
            size: Decimal::from_str("0.0001").unwrap(),
            price: Decimal::from_str("90000").unwrap(),
            tif: TimeInForce::GoodUntilCancelled,
            client_order_id: None,
            side: Side::Ask,
            order_type: Default::default(),
        };

        let action = OrderActions::BulkPost((
            account,
            vec![(instrument, order_info)],
            ExecutionType::Unmanaged,
        ));

        let json = serde_json::to_string_pretty(&action).unwrap();
        println!("Serialized OrderActions:\n{}", json);
    }
}
