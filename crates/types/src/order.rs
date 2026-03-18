use crate::{
    identifiers::Account,
    keys::{ClientOrderId, ExternalOrderId, Instrument},
    primitive::{Side, TimeInForce},
};
use borsh::{BorshDeserialize, BorshSerialize};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

#[derive(
    Default,
    BorshSerialize,
    BorshDeserialize,
    Debug,
    Clone,
    Copy,
    Hash,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
)]
pub enum OrderType {
    #[default]
    Limit,
    Market,
    Pegged,
}

#[derive(
    Clone, BorshSerialize, BorshDeserialize, Debug, Serialize, Deserialize, PartialEq, Eq, Hash,
)]
pub struct Order {
    pub size: Decimal,
    pub price: Decimal,
    pub tif: TimeInForce,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Serialize, Deserialize)]
pub struct OrderInfo {
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal,
    pub tif: TimeInForce,
    #[serde(default)]
    pub order_type: OrderType,
    pub client_order_id: Option<ClientOrderId>,
}

impl Hash for OrderInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.side.hash(state);
        self.price.hash(state);
        self.size.hash(state);
        self.size.hash(state);
        self.order_type.hash(state);
        self.client_order_id.hash(state);
    }
}

impl PartialEq for OrderInfo {
    fn eq(&self, other: &Self) -> bool {
        let tif_eq = if self.tif == TimeInForce::Unknown || other.tif == TimeInForce::Unknown {
            true
        } else {
            self.tif == other.tif
        };

        self.side == other.side
            && self.price == other.price
            && self.size == other.size
            && tif_eq
            && self.order_type == other.order_type
            && self.client_order_id == other.client_order_id
    }
}
impl Eq for OrderInfo {}

impl OrderInfo {
    pub const fn new(side: Side, price: Decimal, size: Decimal, tif: TimeInForce) -> Self {
        Self {
            side,
            price,
            size,
            tif,
            order_type: OrderType::Limit,
            client_order_id: None,
        }
    }

    pub fn with_client_order_id(mut self, client_order_id: ClientOrderId) -> Self {
        self.client_order_id = Some(client_order_id);
        self
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn with_order_type(mut self, order_type: OrderType) -> Self {
        self.order_type = order_type;
        self
    }
}

#[derive(
    BorshSerialize, BorshDeserialize, Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize,
)]
pub struct OpenOrder {
    pub info: OrderInfo,
    pub original_size: Decimal,
    pub is_snapshot: bool,
}

#[derive(
    Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize, Hash,
)]
pub enum OrderReference {
    ExternalOrderId(ExternalOrderId),
    ClientOrderId(ClientOrderId),
}

impl From<String> for OrderReference {
    fn from(s: String) -> Self {
        Self::ExternalOrderId(s.into())
    }
}

impl From<&str> for OrderReference {
    fn from(s: &str) -> Self {
        Self::ExternalOrderId(s.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum ExecutionType {
    Managed,
    Unmanaged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum OrderActions {
    BulkPost((Account, Vec<(Instrument, OrderInfo)>, ExecutionType)),
    BulkCancel((Account, Vec<(Instrument, OrderReference)>, ExecutionType)),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum BulkPostStatus {
    Resting {
        order_id: String,
    },
    Filled {
        order_id: String,
        size: Decimal,
        average_price: Decimal,
    },
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum BulkCancelStatus {
    Success,
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct BulkPostResult {
    pub instrument: Instrument,
    pub order: OrderInfo,
    pub status: BulkPostStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct BulkCancelResult {
    pub instrument: Instrument,
    pub order_id: OrderReference,
    pub status: BulkCancelStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum OrderbookResults {
    BulkPost(Vec<BulkPostResult>),
    BulkCancel(Vec<BulkCancelResult>),
}
