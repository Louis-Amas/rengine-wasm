use crate::McpState;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rengine_types::{
    Account, Decimal, ExecutionRequest, ExecutionType, Instrument, MarketType, OrderActions,
    OrderInfo, OrderReference, OrderType, Side, TimeInForce, Venue,
};
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;
use std::{collections::HashMap, str::FromStr};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn json_text(json: serde_json::Value) -> String {
    serde_json::to_string_pretty(&json).unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"))
}

fn parse_market_type(s: &str) -> Result<MarketType, String> {
    MarketType::from_str(s).map_err(|e| format!("invalid market_type: {e}"))
}

fn decode_wasm(wasm_base64: &str) -> Result<Vec<u8>, String> {
    BASE64
        .decode(wasm_base64)
        .map_err(|e| format!("invalid base64: {e}"))
}

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FilterByVenueSymbol {
    /// Filter by venue name (e.g. "hyperliquid", "binance")
    venue: Option<String>,
    /// Filter by symbol (e.g. "eth", "usdc")
    symbol: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FilterByVenueInstrument {
    /// Filter by venue name
    venue: Option<String>,
    /// Filter by instrument (e.g. "eth/usdc-spot")
    instrument: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FilterByPrefix {
    /// Filter indicator keys by prefix
    prefix: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PlaceOrdersParams {
    /// Exchange venue (e.g. "hyperliquid")
    venue: String,
    /// Market type: "spot" or "perp"
    market_type: String,
    /// Account identifier
    account_id: String,
    /// JSON array of order objects. Each object: { instrument, side ("Ask"/"Bid"), price (string), size (string), `time_in_force`? ("PostOnly"/"GoodUntilCancelled"/"ImmediateOrCancel"), `order_type`? ("Limit"/"Market"/"Pegged"), `client_order_id`? }
    orders_json: String,
    /// Execution type: "Managed" or "Unmanaged" (default: "Unmanaged")
    execution_type: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CancelOrdersParams {
    /// Exchange venue
    venue: String,
    /// Market type: "spot" or "perp"
    market_type: String,
    /// Account identifier
    account_id: String,
    /// JSON array of cancellation objects. Each: { instrument, `order_id`, `ref_type`? ("external"/"client") }
    cancellations_json: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AddStrategyParams {
    /// Strategy identifier
    id: String,
    /// Base64-encoded WASM binary
    wasm_base64: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ToggleStrategyParams {
    /// Strategy identifier
    id: String,
    /// Whether to enable (true) or disable (false)
    enabled: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExecuteStrategyParams {
    /// Base64-encoded WASM binary
    wasm_base64: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EvmWasmParams {
    /// EVM venue / chain name (e.g. "arbitrum")
    venue: String,
    /// Reader / processor identifier
    id: String,
    /// Base64-encoded WASM binary
    wasm_base64: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EvmIdParams {
    /// EVM venue / chain name
    venue: String,
    /// Reader / processor identifier
    id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BuildComponentParams {
    /// Component type: "strategy", "transformer", "multicall", or `evm_logs`
    component_type: String,
    /// Component name (used as the crate name)
    name: String,
    /// Source files as a map of path to content (e.g. `{"src/lib.rs": "use strategy_api::*; ..."}`)
    files: HashMap<String, String>,
    /// Optional whitelisted dependencies to include (e.g. `["serde_json", "alloy"]`)
    #[serde(default)]
    dependencies: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CheckComponentParams {
    /// Component type: "strategy", "transformer", "multicall", or `evm_logs`
    component_type: String,
    /// Component name
    name: String,
    /// Source files as a map of path to content
    files: HashMap<String, String>,
    /// Optional whitelisted dependencies
    #[serde(default)]
    dependencies: Vec<String>,
}

// ---------------------------------------------------------------------------
// Internal order deserialization
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OrderInput {
    instrument: String,
    side: Side,
    price: Decimal,
    size: Decimal,
    #[serde(default = "default_tif")]
    time_in_force: TimeInForce,
    #[serde(default)]
    order_type: OrderType,
    client_order_id: Option<String>,
}

const fn default_tif() -> TimeInForce {
    TimeInForce::GoodUntilCancelled
}

#[derive(Deserialize)]
struct CancelInput {
    instrument: String,
    order_id: String,
    #[serde(default = "default_ref_type")]
    ref_type: String,
}

fn default_ref_type() -> String {
    "external".to_string()
}

// ---------------------------------------------------------------------------
// Tool router
// ---------------------------------------------------------------------------

#[tool_router(server_handler)]
impl McpState {
    // =======================================================================
    // State Reading Tools
    // =======================================================================

    #[tool(description = "Get account balances. Optionally filter by venue and/or symbol.")]
    async fn get_balances(
        &self,
        Parameters(params): Parameters<FilterByVenueSymbol>,
    ) -> Result<String, String> {
        let state = self.state.read();
        let filtered: serde_json::Map<String, serde_json::Value> = state
            .balances
            .iter()
            .filter(|(key, _)| {
                params
                    .venue
                    .as_ref()
                    .is_none_or(|v| key.account.venue.as_str() == v)
                    && params
                        .symbol
                        .as_ref()
                        .is_none_or(|s| key.symbol.as_str() == s)
            })
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string())))
            .collect();

        Ok(json_text(serde_json::Value::Object(filtered)))
    }

    #[tool(description = "Get open positions. Optionally filter by venue and/or instrument.")]
    async fn get_positions(
        &self,
        Parameters(params): Parameters<FilterByVenueInstrument>,
    ) -> Result<String, String> {
        let state = self.state.read();
        let filtered: serde_json::Map<String, serde_json::Value> = state
            .positions
            .iter()
            .filter(|(key, _)| {
                params
                    .venue
                    .as_ref()
                    .is_none_or(|v| key.account.venue.as_str() == v)
                    && params
                        .instrument
                        .as_ref()
                        .is_none_or(|i| key.instrument.as_str() == i)
            })
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string())))
            .collect();

        Ok(json_text(serde_json::Value::Object(filtered)))
    }

    #[tool(
        description = "Get top-of-book (best bid/ask) for instruments. Optionally filter by venue and/or instrument."
    )]
    async fn get_order_book(
        &self,
        Parameters(params): Parameters<FilterByVenueInstrument>,
    ) -> Result<String, String> {
        let state = self.state.read();
        let entries: Vec<serde_json::Value> = state
            .book
            .iter()
            .filter(|(key, _)| {
                params.venue.as_ref().is_none_or(|v| key.venue.as_str() == v)
                    && params
                        .instrument
                        .as_ref()
                        .is_none_or(|i| key.instrument.as_str() == i)
            })
            .map(|(key, book)| {
                serde_json::json!({
                    "key": key.to_string(),
                    "top_bid": { "price": book.top_bid.price.to_string(), "size": book.top_bid.size.to_string() },
                    "top_ask": { "price": book.top_ask.price.to_string(), "size": book.top_ask.size.to_string() },
                    "mid": book.mid().to_string(),
                })
            })
            .collect();

        Ok(json_text(serde_json::json!(entries)))
    }

    #[tool(description = "Get open orders. Optionally filter by venue and/or instrument.")]
    async fn get_open_orders(
        &self,
        Parameters(params): Parameters<FilterByVenueInstrument>,
    ) -> Result<String, String> {
        let state = self.state.read();
        let filtered: serde_json::Map<String, serde_json::Value> = state
            .open_orders
            .iter()
            .filter(|(key, _)| {
                params
                    .venue
                    .as_ref()
                    .is_none_or(|v| key.account.venue.as_str() == v)
                    && params
                        .instrument
                        .as_ref()
                        .is_none_or(|i| key.instrument.as_str() == i)
            })
            .map(|(k, orders)| {
                let orders_json = serde_json::to_value(orders).unwrap_or_default();
                (k.to_string(), orders_json)
            })
            .collect();

        Ok(json_text(serde_json::Value::Object(filtered)))
    }

    #[tool(description = "Get indicator values. Optionally filter by key prefix.")]
    async fn get_indicators(
        &self,
        Parameters(params): Parameters<FilterByPrefix>,
    ) -> Result<String, String> {
        let state = self.state.read();
        let filtered: serde_json::Map<String, serde_json::Value> = state
            .indicators
            .iter()
            .filter(|(key, _)| {
                params
                    .prefix
                    .as_ref()
                    .is_none_or(|p| key.as_str().starts_with(p.as_str()))
            })
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string())))
            .collect();

        Ok(json_text(serde_json::Value::Object(filtered)))
    }

    #[tool(
        description = "Get market specifications (size/price decimals, increments, limits). Optionally filter by venue and/or instrument."
    )]
    async fn get_market_specs(
        &self,
        Parameters(params): Parameters<FilterByVenueInstrument>,
    ) -> Result<String, String> {
        let state = self.state.read();
        let entries: Vec<serde_json::Value> = state
            .market_specs
            .iter()
            .filter(|(key, _)| {
                params
                    .venue
                    .as_ref()
                    .is_none_or(|v| key.venue.as_str() == v)
                    && params
                        .instrument
                        .as_ref()
                        .is_none_or(|i| key.instrument.as_str() == i)
            })
            .map(|(key, spec)| {
                let mut val = serde_json::to_value(spec).unwrap_or_default();
                if let serde_json::Value::Object(ref mut m) = val {
                    m.insert(
                        "key".to_string(),
                        serde_json::Value::String(key.to_string()),
                    );
                }
                val
            })
            .collect();

        Ok(json_text(serde_json::json!(entries)))
    }

    // =======================================================================
    // Order Tools
    // =======================================================================

    #[tool(
        description = "Place one or more orders on an exchange. The orders_json field is a JSON array string of order objects, each with: instrument (str), side (\"Ask\"/\"Bid\"), price (decimal str), size (decimal str), time_in_force? (\"PostOnly\"/\"GoodUntilCancelled\"/\"ImmediateOrCancel\"), order_type? (\"Limit\"/\"Market\"/\"Pegged\"), client_order_id? (str)."
    )]
    async fn place_orders(
        &self,
        Parameters(params): Parameters<PlaceOrdersParams>,
    ) -> Result<String, String> {
        let mt = parse_market_type(&params.market_type)?;
        let account = Account {
            venue: Venue::from(params.venue.as_str()),
            market_type: mt,
            account_id: params.account_id.into(),
        };

        let orders: Vec<OrderInput> = serde_json::from_str(&params.orders_json)
            .map_err(|e| format!("invalid orders_json: {e}"))?;

        let order_pairs: Vec<(Instrument, OrderInfo)> = orders
            .into_iter()
            .map(|o| {
                let mut info = OrderInfo::new(o.side, o.price, o.size, o.time_in_force);
                info = info.with_order_type(o.order_type);
                if let Some(cid) = o.client_order_id {
                    info = info.with_client_order_id(cid.into());
                }
                (Instrument::from(o.instrument.as_str()), info)
            })
            .collect();

        let exec_type = match params.execution_type.as_deref() {
            Some("Managed") => ExecutionType::Managed,
            _ => ExecutionType::Unmanaged,
        };

        let action = OrderActions::BulkPost((account, order_pairs, exec_type));
        let request = ExecutionRequest::Orderbook(action);

        self.external_requests_tx
            .send(request)
            .map_err(|e| format!("failed to send order: {e}"))?;

        Ok(json_text(serde_json::json!({ "status": "submitted" })))
    }

    #[tool(
        description = "Cancel one or more orders. The cancellations_json field is a JSON array string of objects, each with: instrument (str), order_id (str), ref_type? (\"external\"/\"client\", default \"external\")."
    )]
    async fn cancel_orders(
        &self,
        Parameters(params): Parameters<CancelOrdersParams>,
    ) -> Result<String, String> {
        let mt = parse_market_type(&params.market_type)?;
        let account = Account {
            venue: Venue::from(params.venue.as_str()),
            market_type: mt,
            account_id: params.account_id.into(),
        };

        let cancels: Vec<CancelInput> = serde_json::from_str(&params.cancellations_json)
            .map_err(|e| format!("invalid cancellations_json: {e}"))?;

        let cancel_pairs: Vec<(Instrument, OrderReference)> = cancels
            .into_iter()
            .map(|c| {
                let reference = if c.ref_type == "client" {
                    OrderReference::ClientOrderId(c.order_id.into())
                } else {
                    OrderReference::ExternalOrderId(c.order_id.into())
                };
                (Instrument::from(c.instrument.as_str()), reference)
            })
            .collect();

        let action = OrderActions::BulkCancel((account, cancel_pairs, ExecutionType::Unmanaged));
        let request = ExecutionRequest::Orderbook(action);

        self.external_requests_tx
            .send(request)
            .map_err(|e| format!("failed to send cancel: {e}"))?;

        Ok(json_text(serde_json::json!({ "status": "submitted" })))
    }

    // =======================================================================
    // Strategy Management Tools
    // =======================================================================

    #[tool(description = "Add a WASM strategy to the engine. It will be enabled immediately.")]
    async fn add_strategy(
        &self,
        Parameters(params): Parameters<AddStrategyParams>,
    ) -> Result<String, String> {
        let wasm = decode_wasm(&params.wasm_base64)?;

        self.strategies_handler
            .add(params.id.into(), &wasm, true)
            .await
            .map_err(|e| e.to_string())?;

        Ok(json_text(serde_json::json!({ "status": "ok" })))
    }

    #[tool(description = "Enable or disable an existing strategy.")]
    async fn toggle_strategy(
        &self,
        Parameters(params): Parameters<ToggleStrategyParams>,
    ) -> Result<String, String> {
        self.strategies_handler
            .set_enabled(params.id.into(), params.enabled)
            .await
            .map_err(|e| e.to_string())?;

        Ok(json_text(serde_json::json!({ "status": "ok" })))
    }

    #[tool(
        description = "Execute a one-off WASM strategy without persisting it. Returns execution result with emitted requests and logs."
    )]
    async fn execute_strategy(
        &self,
        Parameters(params): Parameters<ExecuteStrategyParams>,
    ) -> Result<String, String> {
        let wasm = decode_wasm(&params.wasm_base64)?;

        let res = self
            .strategies_handler
            .instantiate_and_execute(&wasm)
            .await
            .map_err(|e| e.to_string())?;

        let json = serde_json::to_value(res).map_err(|e| format!("serialization error: {e}"))?;
        Ok(json_text(json))
    }

    // =======================================================================
    // EVM Reader Management Tools
    // =======================================================================

    #[tool(description = "Add an EVM multicall reader to a venue.")]
    async fn add_multicall_reader(
        &self,
        Parameters(params): Parameters<EvmWasmParams>,
    ) -> Result<String, String> {
        let venue_key: Venue = params.venue.as_str().into();
        let wasm = decode_wasm(&params.wasm_base64)?;

        let reader = self
            .evm_readers
            .get(&venue_key)
            .ok_or_else(|| format!("evm reader not found for venue: {}", params.venue))?;

        reader
            .add_multicall_reader(params.id.into(), wasm)
            .await
            .map_err(|e| e.to_string())?;

        Ok(json_text(serde_json::json!({ "status": "ok" })))
    }

    #[tool(description = "Remove an EVM multicall reader from a venue.")]
    async fn remove_multicall_reader(
        &self,
        Parameters(params): Parameters<EvmIdParams>,
    ) -> Result<String, String> {
        let venue_key: Venue = params.venue.as_str().into();

        let reader = self
            .evm_readers
            .get(&venue_key)
            .ok_or_else(|| format!("evm reader not found for venue: {}", params.venue))?;

        reader
            .remove_multicall_reader(params.id.into())
            .await
            .map_err(|e| e.to_string())?;

        Ok(json_text(serde_json::json!({ "status": "ok" })))
    }

    #[tool(description = "Add an EVM log processor to a venue.")]
    async fn add_log_processor(
        &self,
        Parameters(params): Parameters<EvmWasmParams>,
    ) -> Result<String, String> {
        let venue_key: Venue = params.venue.as_str().into();
        let wasm = decode_wasm(&params.wasm_base64)?;

        let reader = self
            .evm_readers
            .get(&venue_key)
            .ok_or_else(|| format!("evm reader not found for venue: {}", params.venue))?;

        reader
            .add_log_reader(params.id.clone(), wasm)
            .await
            .map_err(|e| e.to_string())?;

        Ok(json_text(serde_json::json!({ "status": "ok" })))
    }

    #[tool(description = "Remove an EVM log processor from a venue.")]
    async fn remove_log_processor(
        &self,
        Parameters(params): Parameters<EvmIdParams>,
    ) -> Result<String, String> {
        let venue_key: Venue = params.venue.as_str().into();

        let reader = self
            .evm_readers
            .get(&venue_key)
            .ok_or_else(|| format!("evm reader not found for venue: {}", params.venue))?;

        reader
            .remove_log_reader(params.id.clone())
            .await
            .map_err(|e| e.to_string())?;

        Ok(json_text(serde_json::json!({ "status": "ok" })))
    }

    // =======================================================================
    // WASM Builder Tools
    // =======================================================================

    #[tool(
        description = "Compile a WASM component from Rust source code. Returns base64-encoded WASM binary on success, or build logs on failure. Supported component types: strategy, transformer, multicall, evm_logs. Allowed dependencies: borsh, rust_decimal, rust_decimal_macros, serde, serde_json, anyhow, smol_str, alloy."
    )]
    async fn build_component(
        &self,
        Parameters(params): Parameters<BuildComponentParams>,
    ) -> Result<String, String> {
        let url = self
            .wasm_builder_url
            .as_ref()
            .ok_or("wasm_builder_url is not configured")?;

        let resp = self
            .http_client
            .post(format!("{url}/build"))
            .json(&serde_json::json!({
                "component_type": params.component_type,
                "name": params.name,
                "files": params.files,
                "dependencies": params.dependencies,
            }))
            .send()
            .await
            .map_err(|e| format!("request to wasm-builder failed: {e}"))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("failed to parse wasm-builder response: {e}"))?;

        Ok(json_text(body))
    }

    #[tool(
        description = "Check (type-check) a WASM component without full compilation. Returns structured diagnostics with file, line, column, severity, and error codes. Useful for fast feedback on code correctness."
    )]
    async fn check_component(
        &self,
        Parameters(params): Parameters<CheckComponentParams>,
    ) -> Result<String, String> {
        let url = self
            .wasm_builder_url
            .as_ref()
            .ok_or("wasm_builder_url is not configured")?;

        let resp = self
            .http_client
            .post(format!("{url}/check"))
            .json(&serde_json::json!({
                "component_type": params.component_type,
                "name": params.name,
                "files": params.files,
                "dependencies": params.dependencies,
            }))
            .send()
            .await
            .map_err(|e| format!("request to wasm-builder failed: {e}"))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("failed to parse wasm-builder response: {e}"))?;

        Ok(json_text(body))
    }
}
