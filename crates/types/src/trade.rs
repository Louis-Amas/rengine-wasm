use crate::{
    identifiers::{Account, VenueBookKey},
    keys::{OrderId, Symbol, TradeId},
    primitive::{MarketType, Side, Timestamp},
};
use borsh::{BorshDeserialize, BorshSerialize};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct PublicTrade {
    pub price: Decimal,
    pub size: Decimal,
    pub side: Side,
    pub time: u64,
    pub trade_id: String,
    pub book_key: VenueBookKey,
}

#[derive(Deserialize, Serialize, Clone, Debug, BorshSerialize, BorshDeserialize, Default)]
pub struct PublicTrades {
    pub data: Vec<PublicTrade>,
}

#[derive(Clone, Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Trade {
    pub emitted_at: Timestamp,
    pub received_at: Timestamp,
    pub order_id: OrderId,
    pub trade_id: TradeId,
    pub account: Account,
    pub base: Symbol,
    pub quote: Symbol,
    pub side: Side,
    pub market_type: MarketType,
    pub price: Decimal,
    pub size: Decimal,
    pub fee: Decimal,
    pub fee_symbol: Symbol,
}
