use chrono::{DateTime, Utc};
use rengine_types::{Instrument, OrderInfo, OrderReference, Side, TimeInForce};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use strum::Display;

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
    pub id: String,
    #[serde(flatten)]
    pub payload: ActionPayload<T>,
}

impl<T> BinanceWsRequest<T> {
    pub const fn new(id: String, payload: ActionPayload<T>) -> Self {
        Self { id, payload }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinanceOrderStatus {
    New,
    Canceled,
    Filled,
    PartiallyFilled,
    Expired,
    Rejected,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceOrderResponse {
    pub client_order_id: String,
    pub orig_client_order_id: Option<String>,
    pub order_id: i64,
    pub order_list_id: i64,
    pub symbol: String,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub transact_time: DateTime<Utc>,
    pub status: BinanceOrderStatus,
    pub price: Decimal,
    pub orig_qty: Decimal,
    pub executed_qty: Decimal,
    pub cummulative_quote_qty: Decimal,
    pub time_in_force: BinanceTimeInForce,
    pub r#type: BinanceOrderType,
    pub side: BinanceSide,
}

impl BinanceOrderResponse {
    pub fn get_client_order_id(&self) -> String {
        match self.status {
            BinanceOrderStatus::New => self.client_order_id.clone(),
            BinanceOrderStatus::Canceled
            | BinanceOrderStatus::Filled
            | BinanceOrderStatus::PartiallyFilled
            | BinanceOrderStatus::Expired
            | BinanceOrderStatus::Rejected => self
                .orig_client_order_id
                .clone()
                .unwrap_or_else(|| self.client_order_id.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BinanceIncomingMsg {
    Success {
        id: String,
        status: i64,
        result: BinanceOrderResponse,
    },
    Error {
        id: String,
        status: i64,
        error: BinanceApiError,
    },
}

impl BinanceIncomingMsg {
    pub fn event_id(&self) -> String {
        match self {
            Self::Success { id, .. } | Self::Error { id, .. } => id,
        }
        .clone()
    }
}

#[derive(Debug, Deserialize)]
pub struct BinanceApiError {
    pub code: i64,
    pub msg: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinanceOrderType {
    Limit,
    Market,
    StopLoss,
    StopLossLimit,
    TakeProfit,
    TakeProfitLimit,
    LimitMaker,
    Pegged,
}

impl BinanceOrderType {
    pub const fn from_tif(tif: &TimeInForce) -> Self {
        match tif {
            TimeInForce::PostOnly => Self::LimitMaker,
            _ => Self::Limit,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinanceTimeInForce {
    Gtc,
    Ioc,
    Fok,
    Gtx,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinancePegPriceType {
    PrimaryPeg,
}

impl BinanceTimeInForce {
    pub fn from_tif_and_order_type(
        tif: TimeInForce,
        order_type: &BinanceOrderType,
    ) -> Option<Self> {
        match order_type {
            BinanceOrderType::Limit
            | BinanceOrderType::StopLossLimit
            | BinanceOrderType::TakeProfitLimit
            | BinanceOrderType::Pegged => Some(tif.into()),
            _ => None,
        }
    }
}

impl From<TimeInForce> for BinanceTimeInForce {
    fn from(value: TimeInForce) -> Self {
        match value {
            TimeInForce::GoodUntilCancelled | TimeInForce::Unknown | TimeInForce::ReduceOnly => {
                Self::Gtc
            }
            TimeInForce::ImmediateOrCancel => Self::Ioc,
            TimeInForce::PostOnly => Self::Gtx,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceCreateOrderReq {
    pub api_key: String,
    pub symbol: Instrument,
    pub side: String,
    pub r#type: BinanceOrderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<BinanceTimeInForce>,
    pub quantity: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peg_price_type: Option<BinancePegPriceType>,
    pub new_client_order_id: String,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub timestamp: DateTime<Utc>,
    pub new_order_resp_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl BinanceCreateOrderReq {
    pub fn from_order(instrument: Instrument, order: OrderInfo, api_key: String) -> Self {
        let side = match order.side {
            Side::Bid => "BUY",
            Side::Ask => "SELL",
        };
        let is_pegged = order.order_type == rengine_types::OrderType::Pegged;
        let order_type = match order.order_type {
            rengine_types::OrderType::Limit => BinanceOrderType::from_tif(&order.tif),
            rengine_types::OrderType::Market => BinanceOrderType::Market,
            // Pegged orders use LIMIT type with peg_price_type field
            rengine_types::OrderType::Pegged => BinanceOrderType::Limit,
        };
        let time_in_force = BinanceTimeInForce::from_tif_and_order_type(order.tif, &order_type);
        let (price, peg_price_type) = if is_pegged {
            (None, Some(BinancePegPriceType::PrimaryPeg))
        } else {
            (Some(order.price), None)
        };

        Self {
            api_key,
            symbol: instrument,
            side: side.to_string(),
            r#type: order_type,
            time_in_force,
            quantity: order.size,
            price,
            stop_price: None,
            peg_price_type,
            new_client_order_id: order
                .client_order_id
                .map(|c| c.to_string())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            timestamp: Utc::now(),
            new_order_resp_type: "RESULT".to_string(),
            signature: None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceCancelOrderReq {
    pub api_key: String,
    pub symbol: Instrument,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig_client_order_id: Option<String>,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl BinanceCancelOrderReq {
    pub fn from_cancel(instrument: Instrument, order_ref: OrderReference, api_key: String) -> Self {
        let (order_id, orig_client_order_id) = match order_ref {
            OrderReference::ExternalOrderId(oid) => (oid.parse().ok(), None),
            OrderReference::ClientOrderId(cid) => (None, Some(cid.to_string())),
        };

        Self {
            api_key,
            symbol: instrument,
            order_id,
            orig_client_order_id,
            timestamp: Utc::now(),
            signature: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinanceSide {
    Buy,
    Sell,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReport {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "c")]
    pub client_order_id: String,
    #[serde(rename = "S")]
    pub side: BinanceSide,
    #[serde(rename = "o")]
    pub order_type: BinanceOrderType,
    #[serde(rename = "f")]
    pub time_in_force: BinanceTimeInForce,
    #[serde(rename = "q")]
    pub quantity: Decimal,
    #[serde(rename = "p")]
    pub price: Decimal,
    #[serde(rename = "X")]
    pub current_order_status: BinanceOrderStatus,
    #[serde(rename = "i")]
    pub order_id: u64,
    #[serde(rename = "l")]
    pub last_executed_quantity: Decimal,
    #[serde(rename = "z")]
    pub cumulative_filled_quantity: Decimal,
    #[serde(rename = "L")]
    pub last_executed_price: Decimal,
    #[serde(rename = "n")]
    pub commission_amount: Option<Decimal>,
    #[serde(rename = "N")]
    pub commission_asset: Option<String>,
    #[serde(rename = "T")]
    pub transaction_time: u64,
    #[serde(rename = "t")]
    pub trade_id: i64,
}

#[derive(Debug, Clone)]
pub enum ExtraData {
    Order(Instrument, OrderInfo),
    Cancel(Instrument, OrderReference),
}
