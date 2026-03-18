use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceContractSymbol {
    pub base_asset: String,
    pub quote_asset: String,
    pub filters: Vec<SymbolFilter>,
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
    #[serde(rename = "NOTIONAL")]
    Notional {
        #[serde(rename = "minNotional")]
        min_notional: Decimal,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct BinanceInstrumentsPayload {
    pub symbols: Vec<BinanceContractSymbol>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepthUpdate {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "U")]
    pub first_update_id: u64,
    #[serde(rename = "u")]
    pub last_update_id: u64,
    #[serde(rename = "pu")]
    pub previous_update_id: Option<u64>,
    #[serde(rename = "b")]
    pub bids: Vec<[Decimal; 2]>,
    #[serde(rename = "a")]
    pub asks: Vec<[Decimal; 2]>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceDepth {
    pub asks: Vec<[Decimal; 2]>,
    pub bids: Vec<[Decimal; 2]>,
    pub last_update_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct BinanceBookTicker {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "b")]
    pub bid_px: Decimal,
    #[serde(rename = "B")]
    pub best_bid_qty: Decimal,
    #[serde(rename = "a")]
    pub ask_px: Decimal,
    #[serde(rename = "A")]
    pub best_ask_qty: Decimal,
    #[serde(rename = "u")]
    pub update_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct SubscriptionResponse {
    pub result: Option<serde_json::Value>,
    pub id: u64,
}

/// Aggregate trade stream data
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
pub enum BinanceSpotPublicMessage {
    AggTrade(BinanceAggTrade),
    DepthUpdate(DepthUpdate),
    BookTicker(BinanceBookTicker),
    SubscriptionResponse(SubscriptionResponse),
}
