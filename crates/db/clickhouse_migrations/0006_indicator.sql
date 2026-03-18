-- =======================
-- Table: indicator
-- =======================
CREATE TABLE IF NOT EXISTS indicator (
    set_at   DateTime64(9, 'UTC') NOT NULL,
    key      String        NOT NULL,
    value    Decimal(38,18) NOT NULL
)
ENGINE = MergeTree()
ORDER BY (key, set_at)
PARTITION BY toYYYYMMDD(set_at);

