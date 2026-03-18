use smol_str::SmolStr;

pub type SharedStr = SmolStr;

pub type IndicatorKey = SharedStr;
pub type StorageKey = SharedStr;
pub type Instrument = SharedStr;
pub type StrategyId = SharedStr;
pub type TransformerId = SharedStr;
pub type MultiCallId = SharedStr;
pub type Symbol = SharedStr;
pub type Venue = SharedStr;
pub type AccountId = SharedStr;
pub type ClientOrderId = SharedStr;
pub type ExternalOrderId = SharedStr;
pub type OrderId = i64;
pub type TradeId = i64;
