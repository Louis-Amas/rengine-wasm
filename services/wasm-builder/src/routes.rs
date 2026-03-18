use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;

use crate::build::{self, BuildRequest};
use crate::check;

#[derive(Clone)]
pub struct AppState {
    pub deps_dir: String,
    pub target_dir: String,
    pub cancel: CancellationToken,
}

impl AppState {
    pub fn new(deps_dir: String, target_dir: String, cancel: CancellationToken) -> Self {
        Self { deps_dir, target_dir, cancel }
    }
}

#[derive(Deserialize)]
pub struct BuildPayload {
    pub component_type: String,
    pub name: String,
    pub files: HashMap<String, String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Serialize)]
pub struct BuildResponse {
    pub success: bool,
    pub wasm_base64: Option<String>,
    pub build_log: String,
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn build_component(
    State(state): State<AppState>,
    Json(payload): Json<BuildPayload>,
) -> impl IntoResponse {
    let req = BuildRequest {
        component_type: payload.component_type,
        name: payload.name,
        files: payload.files,
        dependencies: payload.dependencies,
    };

    let result = build::execute_build(&state.deps_dir, &state.target_dir, &req, &state.cancel).await;

    let response = BuildResponse {
        success: result.success,
        wasm_base64: result.wasm_bytes.map(|b| BASE64.encode(&b)),
        build_log: result.build_log,
    };

    (StatusCode::OK, Json(response))
}

async fn check_component(
    State(state): State<AppState>,
    Json(payload): Json<BuildPayload>,
) -> impl IntoResponse {
    let req = BuildRequest {
        component_type: payload.component_type,
        name: payload.name,
        files: payload.files,
        dependencies: payload.dependencies,
    };

    let result = check::execute_check(&state.deps_dir, &state.target_dir, &req, &state.cancel).await;

    (StatusCode::OK, Json(result))
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/build", post(build_component))
        .route("/check", post(check_component))
        .layer(CorsLayer::permissive())
        .with_state(state)
}
