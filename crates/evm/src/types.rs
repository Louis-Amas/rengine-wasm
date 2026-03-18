use alloy::{consensus::BlockHeader, primitives::B256, rpc::types::Header};
use anyhow::Result;
use chrono::{TimeZone, Utc};
use rengine_types::Timestamp;
use serde::{Deserialize, Deserializer};

fn from_hex_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let s = s.trim_start_matches("0x");
    u64::from_str_radix(s, 16)
        .map_err(|e| serde::de::Error::custom(format!("invalid hex u64: {e}")))
}

fn from_hex_to_datetime<'de, D>(deserializer: D) -> Result<Timestamp, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let s = s.trim_start_matches("0x");
    let timestamp = u64::from_str_radix(s, 16)
        .map_err(|e| serde::de::Error::custom(format!("invalid hex timestamp: {e}")))?;

    Utc.timestamp_opt(timestamp as i64, 0)
        .single()
        .map(Into::into)
        .ok_or_else(|| serde::de::Error::custom("invalid timestamp"))
}

/// Simplified subscription result for `newHeads`
#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct NewHead {
    #[serde(deserialize_with = "from_hex_to_u64")]
    pub number: u64,
    pub hash: B256,
    #[serde(deserialize_with = "from_hex_to_datetime")]
    pub timestamp: Timestamp,
}

impl From<Header> for NewHead {
    fn from(value: Header) -> Self {
        Self {
            number: value.number(),
            hash: value.hash,
            timestamp: Utc
                .timestamp_opt(value.timestamp.try_into().unwrap(), 0)
                .single()
                .map(Into::into)
                .expect("invalid timestamp"),
        }
    }
}
