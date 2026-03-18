-- =======================
-- Table: transformer_log
-- =======================
CREATE TABLE IF NOT EXISTS transformer_log (
    emitted_at      DateTime64(9, 'UTC') NOT NULL,
    transformer_id  String        NOT NULL,
    logs            String        NOT NULL,
    requests        JSON          NOT NULL
)
ENGINE = MergeTree()
ORDER BY (transformer_id, emitted_at)
PARTITION BY toYYYYMMDD(emitted_at)
TTL emitted_at + INTERVAL 30 DAY;
