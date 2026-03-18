use anyhow::{anyhow, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use chrono::{DateTime, TimeZone, Utc};
use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize, Serializer};
use std::{
    fmt,
    io::{self, Read, Write},
    str::FromStr,
};

#[derive(
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
pub enum Side {
    Ask,
    Bid,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Ask => "ask",
            Self::Bid => "bid",
        };
        write!(f, "{value}")
    }
}

#[derive(
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
    PartialOrd,
    Ord,
)]
#[serde(rename_all = "camelCase")]
pub enum MarketType {
    Spot,
    Perp,
}

impl fmt::Display for MarketType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Spot => "spot",
            Self::Perp => "perp",
        };
        write!(f, "{value}")
    }
}

impl FromStr for MarketType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "spot" => Ok(Self::Spot),
            "perp" => Ok(Self::Perp),
            other => Err(anyhow!("unknown market type: `{other}`")),
        }
    }
}

#[derive(
    BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
)]
pub enum OrderStatus {
    Open,
    PartiallyFilled,
    Filled,
    Canceled,
    Unknown,
}

#[derive(
    BorshSerialize, BorshDeserialize, Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize,
)]
pub enum TimeInForce {
    PostOnly,
    GoodUntilCancelled,
    ImmediateOrCancel,
    ReduceOnly,
    Unknown,
}

#[derive(Copy, Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub struct Timestamp(DateTime<Utc>);

impl Timestamp {
    pub fn now() -> Self {
        Self(Utc::now())
    }
}

impl From<DateTime<Utc>> for Timestamp {
    fn from(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }
}

impl From<Timestamp> for DateTime<Utc> {
    fn from(ts: Timestamp) -> Self {
        ts.0
    }
}

impl BorshSerialize for Timestamp {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        BorshSerialize::serialize(&self.0.timestamp(), writer)?;
        BorshSerialize::serialize(&self.0.timestamp_subsec_nanos(), writer)
    }
}

impl BorshDeserialize for Timestamp {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        let secs = i64::deserialize_reader(reader)?;
        let nanos = u32::deserialize_reader(reader)?;
        Utc.timestamp_opt(secs, nanos)
            .single()
            .map(Timestamp)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid timestamp"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct HexBytes(pub Vec<u8>);

impl Serialize for HexBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(&self.0)))
    }
}

impl<'de> Deserialize<'de> for HexBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String as Deserialize>::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        hex::decode(s).map(HexBytes).map_err(DeError::custom)
    }
}

impl From<Vec<u8>> for HexBytes {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}
