use crate::{
    Account, AccountId, IndicatorKey, Instrument, OrderId, SharedStr, StorageKey,
    StrategyExecutionResult, StrategyId, Symbol, Timestamp, Trade, TradeId,
    TransformerExecutionResult, TransformerId, Venue,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub mod decimal_as_i128 {
    use anyhow::Result;
    use rust_decimal::Decimal;
    use serde::{self, ser::Error as SerdeError, Deserialize, Deserializer, Serializer};

    /// Convert a Decimal to an i128 scaled by 1e18.
    ///
    /// Example:
    ///   1.234567891011121314 → 1234567891011121314
    ///   -2.5 → -2500000000000000000
    pub const fn decimal_to_i128_scaled(dec: &Decimal) -> Result<i128> {
        // Get the internal integer representation and scale
        let scale = dec.scale();
        let mut mantissa = dec.mantissa();

        // Scale difference (Decimal stores up to 28 places)
        if scale > 18 {
            // too precise — rounding down to 18 decimals
            let diff = scale - 18;
            mantissa /= 10i128.pow(diff);
        } else if scale < 18 {
            // need to scale up
            let diff = 18 - scale;
            mantissa *= 10i128.pow(diff);
        }

        Ok(mantissa)
    }

    /// Convert back from i128 → Decimal with 18-decimals scale.
    pub fn i128_to_decimal_scaled(val: i128) -> Decimal {
        Decimal::from_i128_with_scale(val, 18)
    }

    pub fn serialize<S>(d: &Decimal, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let scaled = decimal_to_i128_scaled(d).map_err(|e| SerdeError::custom(e.to_string()))?;
        s.serialize_i128(scaled)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = i128::deserialize(deserializer)?;
        Ok(i128_to_decimal_scaled(val))
    }
}

#[derive(Clone, Debug)]
pub struct StrategyDb {
    pub strategy_name: String,
    pub wasm: Vec<u8>,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct TransformerDb {
    pub transformer_name: String,
    pub wasm: Vec<u8>,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct MultiCallDb {
    pub name: String,
    pub venue: String,
    pub wasm: Vec<u8>,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopBookDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub received_at: DateTime<Utc>,
    pub venue: Venue,
    pub base: Symbol,
    pub quote: Symbol,
    pub market_type: String,
    #[serde(with = "decimal_as_i128")]
    pub bid_price: Decimal,
    #[serde(with = "decimal_as_i128")]
    pub bid_size: Decimal,
    #[serde(with = "decimal_as_i128")]
    pub ask_price: Decimal,
    #[serde(with = "decimal_as_i128")]
    pub ask_size: Decimal,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct BalanceDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub received_at: DateTime<Utc>,
    pub account: String,
    pub symbol: Symbol,
    #[serde(with = "decimal_as_i128")]
    pub balance: Decimal,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExposureDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub set_at: DateTime<Utc>,
    pub account: String,
    pub symbol: Symbol,
    #[serde(with = "decimal_as_i128")]
    pub balance: Decimal,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct TradeDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub received_at: DateTime<Utc>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub emitted_at: DateTime<Utc>,
    pub order_id: OrderId,
    pub trade_id: TradeId,
    pub account: String,
    pub base: Symbol,
    pub quote: Symbol,
    pub side: String,
    pub market_type: String,
    #[serde(with = "decimal_as_i128")]
    pub price: Decimal,
    #[serde(with = "decimal_as_i128")]
    pub size: Decimal,
    #[serde(with = "decimal_as_i128")]
    pub fee: Decimal,
    pub fee_symbol: Symbol,
}

impl TradeDb {
    pub fn from_trade(trade: Trade) -> Self {
        Self {
            received_at: trade.received_at.into(),
            emitted_at: trade.emitted_at.into(),
            order_id: trade.order_id,
            trade_id: trade.trade_id,
            account: trade.account.to_string(),
            base: trade.base,
            quote: trade.quote,
            side: trade.side.to_string(),
            market_type: trade.market_type.to_string(),
            price: trade.price,
            size: trade.size,
            fee: trade.fee,
            fee_symbol: trade.fee_symbol,
        }
    }
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenOrderDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub received_at: DateTime<Utc>,
    pub order_id: String,
    pub venue: Venue,
    pub base: Symbol,
    pub quote: Symbol,
    pub market_type: String,
    pub side: String,
    pub account_id: AccountId,
    pub time_in_force: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub sent_at: DateTime<Utc>,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct PositionDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub received_at: DateTime<Utc>,
    pub venue: Venue,
    pub symbol: Symbol,
    pub side: String,
    pub account_id: AccountId,
    pub position_type: String,
    #[serde(with = "decimal_as_i128")]
    pub size: Decimal,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndicatorDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub set_at: DateTime<Utc>,
    pub key: IndicatorKey,
    #[serde(with = "decimal_as_i128")]
    pub value: Decimal,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct StrategyLogsDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub emitted_at: DateTime<Utc>,
    pub strategy_id: StrategyId,
    pub logs: String,
    pub requests: String,
}

impl TryFrom<&StrategyExecutionResult> for StrategyLogsDb {
    type Error = anyhow::Error;

    fn try_from(value: &StrategyExecutionResult) -> Result<Self, Self::Error> {
        Ok(Self {
            emitted_at: value.emitted_at,
            strategy_id: value.strategy_id.clone(),
            logs: value.execution_result.logs.join("\n"),
            requests: serde_json::to_string(&json!({
                "result": value.execution_result.requests
            }))?,
        })
    }
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransformerLogsDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub emitted_at: DateTime<Utc>,
    pub transformer_id: TransformerId,
    pub logs: String,
    pub requests: String,
}

impl TryFrom<&TransformerExecutionResult> for TransformerLogsDb {
    type Error = anyhow::Error;

    fn try_from(value: &TransformerExecutionResult) -> Result<Self, Self::Error> {
        Ok(Self {
            emitted_at: value.emitted_at,
            transformer_id: value.transformer_id.clone(),
            logs: value.execution_result.logs.join("\n"),
            requests: serde_json::to_string(&json!({
                "result": value.execution_result.requests
            }))?,
        })
    }
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicTradeDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub received_at: DateTime<Utc>,
    pub venue: Venue,
    pub instrument: Instrument,
    #[serde(with = "decimal_as_i128")]
    pub price: Decimal,
    #[serde(with = "decimal_as_i128")]
    pub size: Decimal,
    pub side: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub time: DateTime<Utc>,
    pub trade_id: String,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmTxDb {
    pub tx_hash: String,
    pub block_number: u64,
    pub nonce: u64,
    pub from: String,
    pub to: String,
    pub value: String,
    pub gas_limit: u64,
    pub gas_price: Option<String>,
    pub max_fee_per_gas: Option<String>,
    pub max_priority_fee_per_gas: Option<String>,
    pub data: String,
    pub transfers: Option<String>,
    pub error: Option<String>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct EvmLogsDb {
    pub name: String,
    pub venue: String,
    pub wasm: Vec<u8>,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct CounterRow {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub recorded_at: DateTime<Utc>,
    pub name: String,
    pub count: u64,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageDb {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub set_at: DateTime<Utc>,
    pub key: StorageKey,
    pub value: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum Record {
    TopBook(TopBookDb),
    Balance(BalanceDb),
    Trade(TradeDb),
    OpenOrder(OpenOrderDb),
    Position(PositionDb),
    Exposure(ExposureDb),
    Indicator(IndicatorDb),
    StrategyLog(StrategyLogsDb),
    TransformerLog(TransformerLogsDb),
    Latency(LatencySnapshotRow),
    Counter(CounterRow),
    Storage(StorageDb),
    PublicTrade(PublicTradeDb),
    EvmTx(EvmTxDb),
}

#[derive(Debug)]
pub struct Exposure {
    pub account: Account,
    pub base: SharedStr,
    pub quote: SharedStr,
    pub base_exposure: Decimal,
    pub quote_exposure: Decimal,
    pub at: Timestamp,
    pub latest_emitted_at: Timestamp,
}

#[derive(Debug)]
pub struct LatencySnapshotDb {
    pub min: u64,
    pub max: u64,
    pub total: u64,
    pub count: u64,
}

#[derive(Clone, Debug, clickhouse::Row, Serialize, Deserialize, PartialEq, Eq)]
pub struct LatencySnapshotRow {
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub recorded_at: DateTime<Utc>,
    pub latency_id: String,
    pub min_us: u64,
    pub max_us: u64,
    pub total_us: u64,
    pub count: u64,
}
