use chrono::{DateTime, Utc};
use rengine_types::{Instrument, OrderInfo, OrderReference, Side, TimeInForce};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinancePerpSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinancePerpOrderType {
    Limit,
    Market,
    Stop,
    StopMarket,
    TakeProfit,
    TakeProfitMarket,
    TrailingStopMarket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinancePerpTimeInForce {
    Gtc,
    Ioc,
    Fok,
    Gtx,
}

#[derive(Debug, Serialize)]
#[serde(tag = "method", content = "params")]
#[serde(rename_all = "camelCase")]
pub enum ActionPayload<T> {
    #[serde(rename = "order.place")]
    Place(T),
    #[serde(rename = "order.cancel")]
    Cancel(T),
}

#[derive(Debug, Serialize)]
pub struct BinanceWsRequest<T> {
    pub id: SmolStr,
    #[serde(flatten)]
    pub payload: ActionPayload<T>,
}

impl<T> BinanceWsRequest<T> {
    pub fn new(id: String, payload: ActionPayload<T>) -> Self {
        Self {
            id: id.into(),
            payload,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinancePerpCreateOrderReq {
    // Fields must be in alphabetical order for signature calculation
    pub api_key: SmolStr,
    pub new_client_order_id: SmolStr,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_match: Option<SmolStr>,
    pub quantity: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recv_window: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduce_only: Option<SmolStr>,
    pub side: BinancePerpSide,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<SmolStr>,
    pub symbol: SmolStr,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<BinancePerpTimeInForce>,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub timestamp: DateTime<Utc>,
    #[serde(rename = "type")]
    pub order_type: BinancePerpOrderType,
}

impl BinancePerpCreateOrderReq {
    pub fn from_order(instrument: Instrument, order: OrderInfo, api_key: String) -> Self {
        let symbol = instrument.to_string().to_uppercase();
        let symbol = if symbol.ends_with("USDT") || symbol.ends_with("BUSD") {
            symbol
        } else {
            symbol + "USDT"
        };

        let side = match order.side {
            Side::Bid => BinancePerpSide::Buy,
            Side::Ask => BinancePerpSide::Sell,
        };

        let is_pegged = order.order_type == rengine_types::OrderType::Pegged;

        let (order_type, time_in_force) = match order.tif {
            TimeInForce::PostOnly => (
                BinancePerpOrderType::Limit,
                Some(BinancePerpTimeInForce::Gtx),
            ),
            TimeInForce::ImmediateOrCancel => (
                BinancePerpOrderType::Limit,
                Some(BinancePerpTimeInForce::Ioc),
            ),
            TimeInForce::GoodUntilCancelled | TimeInForce::Unknown | TimeInForce::ReduceOnly => (
                BinancePerpOrderType::Limit,
                Some(BinancePerpTimeInForce::Gtc),
            ),
        };

        let (price, price_match) = if is_pegged {
            (None, Some(SmolStr::new_static("QUEUE")))
        } else {
            (Some(order.price), None)
        };

        Self {
            api_key: api_key.into(),
            new_client_order_id: order
                .client_order_id
                .map(|c| c.to_string())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
                .into(),
            price,
            price_match,
            quantity: order.size,
            recv_window: Some(20000),
            reduce_only: matches!(order.tif, TimeInForce::ReduceOnly).then(|| "true".into()),
            side,
            signature: None,
            symbol: symbol.into(),
            time_in_force,
            timestamp: Utc::now(),
            order_type,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinancePerpCancelOrderReq {
    // Fields must be in alphabetical order for signature calculation
    pub api_key: SmolStr,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig_client_order_id: Option<SmolStr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recv_window: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<SmolStr>,
    pub symbol: SmolStr,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub timestamp: DateTime<Utc>,
}

impl BinancePerpCancelOrderReq {
    pub fn from_cancel(instrument: Instrument, order_ref: OrderReference, api_key: String) -> Self {
        let symbol = instrument.to_string().to_uppercase();
        let symbol = if symbol.ends_with("USDT") || symbol.ends_with("BUSD") {
            symbol
        } else {
            symbol + "USDT"
        };

        let (order_id, orig_client_order_id) = match order_ref {
            OrderReference::ExternalOrderId(oid) => (oid.parse().ok(), None),
            OrderReference::ClientOrderId(cid) => (None, Some(cid.to_string().into())),
        };

        Self {
            api_key: api_key.into(),
            order_id,
            orig_client_order_id,
            recv_window: Some(20000),
            signature: None,
            symbol: symbol.into(),
            timestamp: Utc::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ExtraData {
    Order(Instrument, OrderInfo),
    Cancel(Instrument, OrderReference),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinanceOrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Canceled,
    Rejected,
    Expired,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceOrderResponse {
    pub order_id: u64,
    pub symbol: String,
    pub status: BinanceOrderStatus,
    pub client_order_id: String,
    pub price: Decimal,
    pub avg_price: Decimal,
    pub orig_qty: Decimal,
    pub executed_qty: Decimal,
    pub cum_qty: Decimal,
    pub time_in_force: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub reduce_only: bool,
    pub close_position: bool,
    pub side: String,
    pub position_side: String,
    pub stop_price: Decimal,
    pub working_type: String,
    pub price_protect: bool,
    pub orig_type: String,
    pub update_time: u64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BinanceIncomingMsg {
    Success {
        id: String,
        status: i64,
        result: serde_json::Value,
    },
    Error {
        id: String,
        status: i64,
        error: BinanceApiError,
    },
}

#[derive(Debug, Deserialize)]
pub struct BinanceApiError {
    pub code: i64,
    pub msg: String,
}

impl BinanceIncomingMsg {
    pub fn event_id(&self) -> String {
        match self {
            Self::Success { id, .. } | Self::Error { id, .. } => id,
        }
        .clone()
    }
}
