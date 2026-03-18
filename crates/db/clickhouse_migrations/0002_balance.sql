-- =======================
-- Table: balance
-- =======================
CREATE TABLE IF NOT EXISTS balance (
    received_at   DateTime64(9, 'UTC') NOT NULL,
    account       String        NOT NULL,
    symbol        String        NOT NULL,
    balance       Decimal(38,18) NOT NULL
)
ENGINE = MergeTree()
ORDER BY (account, symbol, received_at)
PARTITION BY toYYYYMM(received_at)
TTL received_at + INTERVAL 90 DAY;
