use chrono::{serde::ts_milliseconds, DateTime, Utc};
use rengine_types::{
    Account, ClientOrderId, InstrumentDetails, SharedStr, Side, TimeInForce, Trade as EngineTrade,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum Tif {
    Alo,
    #[default]
    Gtc,
    Ioc,
    FrontendMarket, // market orders created through the UI
}

impl From<TimeInForce> for Tif {
    fn from(value: TimeInForce) -> Self {
        match value {
            TimeInForce::GoodUntilCancelled => Self::Gtc,
            TimeInForce::ImmediateOrCancel => Self::Ioc,
            _ => Self::Alo,
        }
    }
}

impl From<Tif> for TimeInForce {
    fn from(value: Tif) -> Self {
        match value {
            Tif::Gtc => Self::GoodUntilCancelled,
            Tif::Alo => Self::PostOnly,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum HyperLiquidSide {
    #[serde(rename = "A")]
    Ask,
    #[serde(rename = "B")]
    Bid,
}

impl From<HyperLiquidSide> for Side {
    fn from(value: HyperLiquidSide) -> Self {
        match value {
            HyperLiquidSide::Ask => Self::Ask,
            HyperLiquidSide::Bid => Self::Bid,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub(crate) struct BookLevel {
    pub px: Decimal,
    pub sz: Decimal,
    // pub n: u64,
}
#[derive(Deserialize, Clone, Debug)]
pub(crate) struct L2BookData {
    pub coin: SharedStr,
    // pub time: u64,
    pub levels: Vec<Vec<BookLevel>>,
}

#[derive(Deserialize, Clone, Debug)]
pub(crate) struct L2Book {
    pub data: L2BookData,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BasicOrder {
    pub coin: SharedStr,
    pub side: HyperLiquidSide,
    pub limit_px: Decimal,
    pub sz: Decimal,
    pub oid: u64,
    // pub timestamp: u64,
    pub orig_sz: Decimal,
    pub cloid: Option<ClientOrderId>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OrderStatus {
    Open,
    Filled,
    Canceled,
    Triggered,
    Rejected,
    MarginCanceled,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrderUpdate {
    pub order: BasicOrder,
    pub status: OrderStatus,
    // pub status_timestamp: u64,
}

#[derive(Deserialize, Clone, Debug)]
pub(crate) struct OrderUpdates {
    pub data: Vec<OrderUpdate>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) enum Direction {
    #[serde(rename = "Open Long")]
    OpenLong,
    #[serde(rename = "Close Short")]
    CloseShort,
    #[serde(rename = "Close Long")]
    CloseLong,
    #[serde(rename = "Open Short")]
    OpenShort,
    #[serde(rename = "Short > Long")]
    ShortGreaterLong,
    #[serde(rename = "Long > Short")]
    LongGreaterShort,
    #[serde(rename = "Buy")]
    Buy,
    #[serde(rename = "Sell")]
    Sell,

    #[serde(other)]
    Unknown,
}

impl Direction {
    pub(crate) const fn is_perp(&self) -> bool {
        matches!(
            self,
            Self::OpenLong
                | Self::CloseShort
                | Self::ShortGreaterLong
                | Self::CloseLong
                | Self::OpenShort
                | Self::LongGreaterShort
        )
    }
}
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Trade {
    pub(crate) coin: SharedStr,
    pub(crate) side: HyperLiquidSide,
    pub(crate) px: Decimal,
    pub(crate) sz: Decimal,
    #[serde(with = "ts_milliseconds")]
    pub(crate) time: DateTime<Utc>,
    pub(crate) hash: String,
    pub(crate) start_position: Decimal,
    pub(crate) dir: Direction,
    pub(crate) closed_pnl: String,
    pub(crate) oid: u64,
    pub(crate) cloid: Option<String>,
    pub(crate) crossed: bool,
    pub(crate) fee: Decimal,
    pub(crate) tid: u64,
}

impl Trade {
    pub(crate) fn to_engine_trade(
        &self,
        instrument: &InstrumentDetails,
        account: Account,
        received_at: DateTime<Utc>,
    ) -> EngineTrade {
        EngineTrade {
            emitted_at: self.time.into(),
            received_at: received_at.into(),
            order_id: self.oid as i64,
            trade_id: self.tid as i64,
            account,
            base: instrument.base.clone(),
            quote: instrument.quote.clone(),
            side: self.side.into(),
            market_type: instrument.market_type,
            price: self.px,
            size: self.sz,
            fee: self.fee,
            fee_symbol: "usdc".into(), // constant in your example
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) enum UserData {
    Fills(Vec<Trade>),
    Funding(serde_json::Value),
    // Liquidation(Liquidation),
    // NonUserCancel(Vec<NonUserCancel>),
}

#[derive(Deserialize, Clone, Debug)]
pub(crate) struct User {
    pub data: UserData,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PositionData {
    pub coin: SharedStr,
    // pub entry_px: Option<Decimal>,
    // pub leverage: Leverage,
    // pub liquidation_px: Option<Decimal>,
    // pub margin_used: String,
    // pub position_value: String,
    // pub return_on_equity: String,
    pub szi: Decimal,
    // pub unrealized_pnl: Decimal,
}

#[derive(Deserialize, Debug)]
pub(crate) struct AssetPosition {
    pub position: PositionData,
    // #[serde(rename = "type")]
    // pub type_string: String,
}

#[derive(Deserialize, Debug, Clone)]
pub(crate) struct HyperliquidTrade {
    pub coin: SharedStr,
    pub side: HyperLiquidSide,
    pub px: Decimal,
    pub sz: Decimal,
    #[serde(with = "ts_milliseconds")]
    pub time: DateTime<Utc>,
    pub tid: u64,
}

#[derive(Deserialize, Debug, Clone)]
pub(crate) struct HyperliquidTrades {
    pub data: Vec<HyperliquidTrade>,
}
