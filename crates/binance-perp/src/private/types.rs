use chrono::{DateTime, Utc};
use rengine_types::primitive::{Side, TimeInForce};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

#[derive(Debug, Deserialize)]
#[serde(tag = "e")]
pub enum BinanceUserDataEvent {
    #[serde(rename = "ACCOUNT_UPDATE")]
    AccountUpdate {
        #[serde(rename = "E", with = "chrono::serde::ts_milliseconds")]
        event_time: DateTime<Utc>,
        #[serde(rename = "a")]
        update_data: AccountUpdateData,
    },

    #[serde(rename = "ORDER_TRADE_UPDATE")]
    OrderTradeUpdate {
        #[serde(rename = "E", with = "chrono::serde::ts_milliseconds")]
        event_time: DateTime<Utc>,
        #[serde(rename = "o")]
        order: Box<OrderTradeUpdateData>,
    },
    #[serde(rename = "TRADE_LITE")]
    TradeLite {},
}

#[derive(Clone, Debug, Deserialize)]
pub struct AccountUpdateData {
    #[serde(rename = "B")]
    pub balances: Vec<BinanceBalance>,
    #[serde(rename = "P")]
    pub positions: Vec<BinancePosition>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BinanceBalance {
    #[serde(rename = "a")]
    pub asset: SmolStr,
    #[serde(rename = "wb")]
    pub wallet_balance: Decimal,
    #[serde(rename = "cw")]
    pub cross_wallet_balance: Decimal,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BinancePosition {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "pa")]
    pub position_amount: Decimal,
    #[serde(rename = "ep")]
    pub entry_price: Decimal,
    #[serde(rename = "up")]
    pub unrealized_pnl: Decimal,
    #[serde(rename = "mt")]
    pub margin_type: BinanceMarginType,
    #[serde(rename = "iw")]
    pub isolated_wallet: Decimal,
    #[serde(rename = "ps")]
    pub position_side: BinancePositionSide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BinanceMarginType {
    Cross,
    Isolated,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum BinancePositionSide {
    Long,
    Short,
    Both,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum BinanceSide {
    Buy,
    Sell,
}

impl From<BinanceSide> for Side {
    fn from(value: BinanceSide) -> Self {
        match value {
            BinanceSide::Buy => Self::Bid,
            BinanceSide::Sell => Self::Ask,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinanceOrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Canceled,
    Expired,
    ExpiredInMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinanceTimeInForce {
    Gtc, // Good Till Cancelled
    Ioc, // Immediate Or Cancel
    Fok, // Fill Or Kill
    Gtx, // Good Till Crossing (Post-only)
}

impl From<BinanceTimeInForce> for TimeInForce {
    fn from(value: BinanceTimeInForce) -> Self {
        match value {
            BinanceTimeInForce::Gtc => Self::GoodUntilCancelled,
            BinanceTimeInForce::Gtx => Self::PostOnly,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinanceOrderType {
    Limit,
    Market,
    Stop,
    StopMarket,
    TakeProfit,
    TakeProfitMarket,
    TrailingStopMarket,
    Liquidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BinanceExecutionType {
    New,
    Canceled,
    Calculated,
    Expired,
    Trade,
    Amendment,
}

#[derive(Clone, Debug, Deserialize)]
pub struct OrderTradeUpdateData {
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
    pub original_quantity: Decimal,
    #[serde(rename = "p")]
    pub original_price: Decimal,
    #[serde(rename = "sp")]
    pub stop_price: Decimal,
    #[serde(rename = "x")]
    pub execution_type: BinanceExecutionType,
    #[serde(rename = "X")]
    pub order_status: BinanceOrderStatus,
    #[serde(rename = "i")]
    pub order_id: u64,
    #[serde(rename = "l")]
    pub last_filled_quantity: Decimal,
    #[serde(rename = "z")]
    pub accumulated_filled_quantity: Decimal,
    #[serde(rename = "L")]
    pub last_filled_price: Decimal,
    #[serde(rename = "N")]
    pub commission_asset: Option<String>,
    #[serde(rename = "n")]
    pub commission: Option<Decimal>,
    #[serde(rename = "T", with = "chrono::serde::ts_milliseconds")]
    pub trade_time: DateTime<Utc>,
    #[serde(rename = "t")]
    pub trade_id: u64,
    #[serde(rename = "m")]
    pub is_maker: bool,
}

#[derive(Debug, Deserialize)]
pub struct BinanceWsApiResponse<T> {
    pub id: String,
    pub status: i64,
    pub result: T,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenKeyResponse {
    pub listen_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BinancePrivateMessage {
    UserData(BinanceUserDataEvent),
    Unknown(serde_json::Value),
}
