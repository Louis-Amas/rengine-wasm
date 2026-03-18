-- =======================
-- Table: storage
-- =======================
CREATE TABLE IF NOT EXISTS storage (
    set_at   DateTime64(9, 'UTC') NOT NULL,
    key      String        NOT NULL,
    value    Array(UInt8)  NOT NULL COMMENT 'Raw bytes'
)
ENGINE = MergeTree()
ORDER BY (key, set_at)
PARTITION BY toYYYYMMDD(set_at);
