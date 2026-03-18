-- =======================
-- Table: latency_snapshot
-- =======================
CREATE TABLE IF NOT EXISTS latency (
    recorded_at DateTime64(9, 'UTC') NOT NULL,
    latency_id  String        NOT NULL,
    min_us      UInt64        NOT NULL,
    max_us      UInt64        NOT NULL,
    total_us    UInt64        NOT NULL,
    count       UInt64        NOT NULL
)
ENGINE = MergeTree()
ORDER BY (latency_id, recorded_at)
PARTITION BY toYYYYMMDD(recorded_at)
TTL recorded_at + INTERVAL 30 DAY;

