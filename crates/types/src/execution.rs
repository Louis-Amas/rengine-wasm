use crate::{
    identifiers::EvmAccount,
    keys::{IndicatorKey, StrategyId, TransformerId},
    order::{OrderActions, OrderbookResults},
    primitive::{HexBytes, Timestamp},
};
use borsh::{BorshDeserialize, BorshSerialize};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum ExecutionRequest {
    Orderbook(OrderActions),
    EvmTx((EvmAccount, HexBytes)),
    SetIndicator(IndicatorKey, Decimal),
    Nothing,
}

#[derive(
    Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub struct ExecutionRequestsWithLogs {
    pub requests: Vec<ExecutionRequest>,
    pub logs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyExecutionResult {
    pub emitted_at: DateTime<Utc>,
    pub strategy_id: StrategyId,
    pub execution_result: ExecutionRequestsWithLogs,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformerExecutionResult {
    pub emitted_at: DateTime<Utc>,
    pub transformer_id: TransformerId,
    pub execution_result: ExecutionRequestsWithLogs,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct TimestampedData<T> {
    pub data: T,
    pub received_at: Timestamp,
    pub emited_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum ExecutionResult {
    Orderbook(TimestampedData<OrderbookResults>),
}
