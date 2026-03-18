use crate::{
    book::TopBookUpdate,
    config::MarketSpec,
    execution::ExecutionResult,
    identifiers::{BalanceKey, BookKey, VenueBookKey},
    keys::{IndicatorKey, StorageKey},
    order::{OpenOrder, OrderInfo},
    trade::Trade,
    PublicTrades,
};
use borsh::{BorshDeserialize, BorshSerialize};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub indicators: HashMap<IndicatorKey, Decimal>,
    pub balances: HashMap<BalanceKey, Decimal>,
    pub book: HashMap<VenueBookKey, TopBookUpdate>,
    pub open_orders: HashMap<BookKey, HashMap<String, OrderInfo>>,
    pub positions: HashMap<BookKey, Decimal>,
    pub spot_exposures: HashMap<BalanceKey, Decimal>,
    /// Market specifications for trading instruments, keyed by `VenueBookKey`
    pub market_specs: HashMap<VenueBookKey, MarketSpec>,
    /// Arbitrary storage for strategies, keyed by `StorageKey`
    pub storage: HashMap<StorageKey, Vec<u8>>,
}

#[derive(Debug, Deserialize, Serialize, BorshSerialize, BorshDeserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    SetBalance(BalanceKey, Decimal),
    SetIndicator(IndicatorKey, Decimal),
    SetTopBook(VenueBookKey, TopBookUpdate),
    SetOpenOrder(BookKey, String, OpenOrder),
    RemoveOpenOrder(BookKey, String),
    UpdateOpenOrder(BookKey, String, Decimal),
    SetPerpPosition(BookKey, Decimal),
    SetTradeFlow(VenueBookKey, PublicTrades),
    RecordTrades(Vec<(BookKey, Trade)>),
    HandleExecutionResult(ExecutionResult),
    /// Set market specification for an instrument
    SetMarketSpec(VenueBookKey, MarketSpec),
    /// Set arbitrary storage data
    SetStorage(StorageKey, Vec<u8>),
}

#[derive(
    Clone,
    Debug,
    Eq,
    Hash,
    PartialEq,
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
    PartialOrd,
    Ord,
)]
pub enum StateUpdateKey {
    SetBalance(BalanceKey),
    SetIndicator(IndicatorKey),
    SetTopBook(VenueBookKey),
    SetPerpOpenOrder(BookKey),
    RemovePerpOpenOrder(BookKey),
    UpdatePerpOpenOrder(BookKey),
    SetPerpPosition(BookKey),
    SetTradeFlow(VenueBookKey),
    SetMarketSpec(VenueBookKey),
    SetStorage(StorageKey),
    Timer {
        #[borsh(
            serialize_with = "crate::serialization::borsh_duration::serialize",
            deserialize_with = "crate::serialization::borsh_duration::deserialize"
        )]
        interval: Duration,
    },
    UtcTimer {
        #[borsh(
            serialize_with = "crate::serialization::borsh_duration::serialize",
            deserialize_with = "crate::serialization::borsh_duration::deserialize"
        )]
        interval: Duration,
    },
    None,
}

impl From<&Action> for StateUpdateKey {
    fn from(value: &Action) -> Self {
        match value {
            Action::SetBalance(key, _) => Self::SetBalance(key.clone()),
            Action::SetIndicator(key, _) => Self::SetIndicator(key.clone()),
            Action::SetTopBook(key, _) => Self::SetTopBook(key.clone()),
            Action::SetOpenOrder(key, _, _) => Self::SetPerpOpenOrder(key.clone()),
            Action::RemoveOpenOrder(key, _) => Self::RemovePerpOpenOrder(key.clone()),
            Action::UpdateOpenOrder(key, _, _) => Self::UpdatePerpOpenOrder(key.clone()),
            Action::SetPerpPosition(key, _) => Self::SetPerpPosition(key.clone()),
            Action::SetTradeFlow(key, _) => Self::SetTradeFlow(key.clone()),
            Action::SetMarketSpec(key, _) => Self::SetMarketSpec(key.clone()),
            Action::SetStorage(key, _) => Self::SetStorage(key.clone()),
            Action::RecordTrades(_) | Action::HandleExecutionResult(_) => Self::None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct SifResult {
    pub condition: bool,
    pub logs: String,
}
