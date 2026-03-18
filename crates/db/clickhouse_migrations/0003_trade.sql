-- =======================
-- Table: trade
-- =======================
CREATE TABLE IF NOT EXISTS trade (
    received_at   DateTime64(9, 'UTC') NOT NULL,
    emitted_at    DateTime64(9, 'UTC'),
    order_id      Int64         NOT NULL,
    trade_id      Int64         NOT NULL,
    account       String        NOT NULL,
    base          String        NOT NULL,
    quote         String        NOT NULL,
    side          String        NOT NULL,
    market_type   String        NOT NULL,
    price         Decimal(38,18) NOT NULL,
    size          Decimal(38,18) NOT NULL,
    fee           Decimal(38,18) NOT NULL,
    fee_symbol    String        NOT NULL
)
ENGINE = MergeTree()
ORDER BY (account, base, quote, received_at)
PARTITION BY toYYYYMMDD(received_at)
TTL received_at + INTERVAL 30 DAY;

