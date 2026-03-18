-- =======================
-- Table: position
-- =======================
CREATE TABLE IF NOT EXISTS position (
    received_at    DateTime64(9, 'UTC') NOT NULL,
    venue          String        NOT NULL,
    symbol         String        NOT NULL,
    side           String        NOT NULL,
    account_id     String        NOT NULL,
    position_type  String        NOT NULL,
    size           Decimal(38,18) NOT NULL
)
ENGINE = ReplacingMergeTree()
ORDER BY (venue, account_id, symbol, received_at)
PARTITION BY toYYYYMM(received_at)
TTL received_at + INTERVAL 90 DAY;
