use borsh::{BorshDeserialize, BorshSerialize};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, BorshSerialize, BorshDeserialize, Debug, Serialize, Deserialize, PartialEq, Eq, Hash,
)]
pub struct Level {
    pub size: Decimal,
    pub price: Decimal,
}

#[derive(Clone, BorshSerialize, BorshDeserialize, Debug, Serialize, Deserialize)]
pub struct TopBookUpdate {
    pub top_ask: Level,
    pub top_bid: Level,
}

impl TopBookUpdate {
    pub fn mid(&self) -> Decimal {
        dec!(0.5) * (self.top_bid.price + self.top_ask.price)
    }
}
