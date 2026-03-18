use crate::execution::types::ExecutionReport;
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct BinanceSpotPrivateMessageWrapper {
    pub event: BinanceSpotPrivateMessage,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BinanceSpotPrivateMessage {
    OrderUpdate(ExecutionReport),
    OutboundAccountPosition(OutboundAccountPosition),
    BalanceUpdate(BalanceUpdate),
    Unknown(serde_json::Value),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboundAccountPosition {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "u")]
    pub last_update_time: u64,
    #[serde(rename = "B")]
    pub balances: Vec<SpotBalance>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceUpdate {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "a")]
    pub asset: String,
    #[serde(rename = "d")]
    pub balance_delta: Decimal,
    #[serde(rename = "T")]
    pub clear_time: u64,
}

#[derive(Debug, Deserialize)]
pub struct SpotBalance {
    #[serde(rename = "a")]
    pub asset: String,
    #[serde(rename = "f")]
    pub free: Decimal,
    #[serde(rename = "l")]
    pub locked: Decimal,
}
