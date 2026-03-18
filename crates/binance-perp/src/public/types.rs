use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceContractSymbol {
    pub symbol: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub contract_type: ContractType,
    pub status: ContractStatus,
    pub filters: Vec<SymbolFilter>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum ContractType {
    Perpetual,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum ContractStatus {
    Trading,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "filterType")]
pub enum SymbolFilter {
    #[serde(rename = "PRICE_FILTER")]
    PriceFilter {
        #[serde(rename = "tickSize")]
        tick_size: Decimal,
    },
    #[serde(rename = "LOT_SIZE")]
    LotSize {
        #[serde(rename = "stepSize")]
        step_size: Decimal,
    },
    #[serde(rename = "MIN_NOTIONAL")]
    MinNotional { notional: Decimal },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct BinanceInstrumentsPayload {
    pub symbols: Vec<BinanceContractSymbol>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceFundingRate {
    pub symbol: String,
    pub funding_rate: Decimal,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub funding_time: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FundingInfo {
    pub symbol: String,
    pub funding_interval_hours: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceDepth {
    pub asks: Vec<[Decimal; 2]>,
    pub bids: Vec<[Decimal; 2]>,
    pub last_update_id: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceDepthWsMessage {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "T")]
    pub transaction_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "U")]
    pub first_update_id: u64,
    #[serde(rename = "u")]
    pub last_update_id: u64,
    #[serde(rename = "pu")]
    pub previous_update_id: u64,
    #[serde(rename = "b")]
    pub bids: Vec<[Decimal; 2]>,
    #[serde(rename = "a")]
    pub asks: Vec<[Decimal; 2]>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BinanceBookTicker {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "u")]
    pub update_id: u64,
    #[serde(rename = "E", with = "chrono::serde::ts_milliseconds")]
    pub event_time: DateTime<Utc>,
    #[serde(rename = "T")]
    pub transaction_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "b")]
    pub bid_price: Decimal,
    #[serde(rename = "B")]
    pub bid_qty: Decimal,
    #[serde(rename = "a")]
    pub ask_price: Decimal,
    #[serde(rename = "A")]
    pub ask_qty: Decimal,
}

/// Aggregate trade stream data for perpetual futures
/// Stream Name: <symbol>@aggTrade
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BinanceAggTrade {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "a")]
    pub aggregate_trade_id: u64,
    #[serde(rename = "p")]
    pub price: Decimal,
    #[serde(rename = "q")]
    pub quantity: Decimal,
    #[serde(rename = "f")]
    pub first_trade_id: u64,
    #[serde(rename = "l")]
    pub last_trade_id: u64,
    #[serde(rename = "T")]
    pub trade_time: u64,
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BinancePublicMessage {
    AggTrade(BinanceAggTrade),
    DepthUpdate(BinanceDepthWsMessage),
    BookTicker(BinanceBookTicker),
    Unknown(serde_json::Value),
}
