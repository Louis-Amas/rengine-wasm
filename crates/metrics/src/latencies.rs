use chrono::Utc;
use once_cell::sync::Lazy;
use rengine_types::db::{LatencySnapshotDb, LatencySnapshotRow};
use scc::HashMap;
use smol_str::SmolStr;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
use strum::{AsRefStr, EnumIter, EnumString, IntoEnumIterator};

/// Latency metric identifiers
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, EnumIter, EnumString, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum LatencyId {
    TimescaleDbFlush,
    HyperLiquidOrderPlace,
    HyperLiquidOrderCancel,
    HyperliquidMarketUpdateWs,
    HandleChanges,
    HandleRequests,
    OuterCrankLoop,
    CrankLoop,
    StrategiesExecution,
    TransformersExecution,
}

#[derive(Debug)]
pub struct LatencyStat {
    pub min: AtomicU64,
    pub max: AtomicU64,
    pub total: AtomicU64,
    pub count: AtomicU64,
}

impl Default for LatencyStat {
    fn default() -> Self {
        Self {
            min: AtomicU64::new(u64::MAX),
            max: <_>::default(),
            total: <_>::default(),
            count: <_>::default(),
        }
    }
}

impl From<&LatencyStat> for LatencySnapshotDb {
    fn from(value: &LatencyStat) -> Self {
        Self {
            min: value.min.load(Ordering::Relaxed),
            max: value.max.load(Ordering::Relaxed),
            total: value.total.load(Ordering::Relaxed),
            count: value.count.load(Ordering::Relaxed),
        }
    }
}

impl From<LatencyId> for SmolStr {
    fn from(id: LatencyId) -> Self {
        Self::from(id.as_ref())
    }
}

impl LatencyStat {
    pub fn record(&self, latency_us: u64) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.total.fetch_add(latency_us, Ordering::Relaxed);

        let mut old_min = self.min.load(Ordering::Relaxed);
        while latency_us < old_min
            && self
                .min
                .compare_exchange(old_min, latency_us, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            old_min = self.min.load(Ordering::Relaxed);
        }

        let mut old_max = self.max.load(Ordering::Relaxed);
        while latency_us > old_max
            && self
                .max
                .compare_exchange(old_max, latency_us, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            old_max = self.max.load(Ordering::Relaxed);
        }
    }

    pub fn reset(&self) {
        self.min.store(u64::MAX, Ordering::Relaxed);
        self.max.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        self.count.store(0, Ordering::Relaxed);
    }
}

/// Static registry of all latency metrics.
pub static LATENCIES: Lazy<HashMap<SmolStr, Arc<LatencyStat>>> = Lazy::new(|| {
    let map = HashMap::new();
    for id in LatencyId::iter() {
        map.insert(SmolStr::from(id.as_ref()), Arc::new(LatencyStat::default()))
            .expect("Failed to insert initial latencies");
    }
    map
});

/// Record a latency duration since `start`.
#[inline(always)]
pub fn record_latency(id: impl Into<SmolStr>, start: Instant) {
    let latency_us = start.elapsed().as_micros() as u64;
    let key = id.into();

    // Fast path: try to get existing
    if let Some(stat) = LATENCIES.get(&key) {
        stat.record(latency_us);
    } else {
        // Slow path: insert if missing
        let stat = Arc::new(LatencyStat::default());
        stat.record(latency_us);
        let _ = LATENCIES.insert(key, stat);
    }
}

/// Record a latency given a raw nanosecond value (converts to microseconds internally).
#[inline(always)]
pub fn record_latency_nanos(id: impl Into<SmolStr>, latency_ns: u64) {
    let latency_us = latency_ns / 1000;
    let key = id.into();

    // Fast path: try to get existing
    if let Some(stat) = LATENCIES.get(&key) {
        stat.record(latency_us);
    } else {
        // Slow path: insert if missing
        let stat = Arc::new(LatencyStat::default());
        stat.record(latency_us);
        let _ = LATENCIES.insert(key, stat);
    }
}

/// Reset all latencies to default.
pub fn reset_all_latencies() {
    LATENCIES.scan(|_k, v| {
        v.reset();
    });
}

pub fn get_all_nonempty_stats() -> Vec<LatencySnapshotRow> {
    let now = Utc::now();
    let mut stats = Vec::new();

    LATENCIES.scan(|k, stat| {
        if stat.count.load(Ordering::Relaxed) > 0 {
            stats.push(LatencySnapshotRow {
                recorded_at: now,
                latency_id: k.to_string(),
                min_us: stat.min.load(Ordering::Relaxed),
                max_us: stat.max.load(Ordering::Relaxed),
                total_us: stat.total.load(Ordering::Relaxed),
                count: stat.count.load(Ordering::Relaxed),
            });
        }
    });

    stats
}
