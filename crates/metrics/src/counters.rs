use once_cell::sync::Lazy;
use scc::HashMap;
use smol_str::SmolStr;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct CounterKey {
    pub name: SmolStr,
}

pub static COUNTERS: Lazy<HashMap<CounterKey, AtomicU64>> = Lazy::new(HashMap::new);

pub fn increment_counter(name: impl Into<SmolStr>) {
    let key = CounterKey { name: name.into() };

    if let Some(val) = COUNTERS.get(&key) {
        val.fetch_add(1, Ordering::Relaxed);
    } else {
        let _ = COUNTERS.insert(key, AtomicU64::new(1));
    }
}

#[derive(Debug)]
pub struct CounterSnapshot {
    pub name: SmolStr,
    pub count: u64,
}

pub fn get_all_counters() -> Vec<CounterSnapshot> {
    let mut snapshots = Vec::new();
    COUNTERS.scan(|k, v| {
        let count = v.load(Ordering::Relaxed);
        if count > 0 {
            snapshots.push(CounterSnapshot {
                name: k.name.clone(),
                count,
            });
        }
    });
    snapshots
}
