-- =======================
-- Table: strategy_log
-- =======================
CREATE TABLE IF NOT EXISTS strategy_log (
    emitted_at   DateTime64(9, 'UTC') NOT NULL,
    strategy_id  String        NOT NULL,
    logs         String        NOT NULL,
    requests     JSON          NOT NULL
)
ENGINE = MergeTree()
ORDER BY (strategy_id, emitted_at)
PARTITION BY toYYYYMMDD(emitted_at)
TTL emitted_at + INTERVAL 30 DAY;

