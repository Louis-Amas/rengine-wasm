CREATE TABLE IF NOT EXISTS counter_metric
(
    `recorded_at` DateTime64 (9, 'UTC'),
    `name` String,
    `count` UInt64
)
ENGINE = MergeTree
PARTITION BY toDate(recorded_at)
ORDER BY (name, recorded_at);