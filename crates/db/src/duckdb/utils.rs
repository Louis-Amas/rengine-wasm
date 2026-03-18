use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use duckdb::{
    types::{TimeUnit, Value},
    Error,
};
use rust_decimal::Decimal;

pub(super) fn value_to_decimal(v: Value) -> Result<Decimal, Error> {
    match v {
        Value::Decimal(d) => Ok(d),
        _ => Err(Error::AppendError),
    }
}

pub(super) fn value_to_datetime(v: Value) -> Result<DateTime<Utc>, Error> {
    match v {
        Value::Timestamp(unit, v) => {
            let total_nanos: i128 = match unit {
                TimeUnit::Second => i128::from(v) * 1_000_000_000,
                TimeUnit::Millisecond => i128::from(v) * 1_000_000,
                TimeUnit::Microsecond => i128::from(v) * 1_000,
                TimeUnit::Nanosecond => i128::from(v),
            };

            let secs = total_nanos.div_euclid(1_000_000_000);
            let nanos = total_nanos.rem_euclid(1_000_000_000) as u32;

            Utc.timestamp_opt(secs as i64, nanos)
                .single()
                .ok_or(Error::AppendError)
        }
        _ => Err(Error::AppendError),
    }
}
