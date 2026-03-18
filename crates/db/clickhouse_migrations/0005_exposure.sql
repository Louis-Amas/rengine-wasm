-- =======================
-- Table: exposure
-- =======================
CREATE TABLE IF NOT EXISTS exposure (
    set_at   DateTime64(9, 'UTC') NOT NULL,
    account  String        NOT NULL,
    symbol   String        NOT NULL,
    balance  Decimal(38,18) NOT NULL
)
ENGINE = MergeTree()
ORDER BY (account, symbol, set_at)
PARTITION BY toYYYYMMDD(set_at);

