-- =======================
-- Table: top_book
-- =======================
CREATE TABLE IF NOT EXISTS top_book (
    received_at   DateTime64(9, 'UTC') NOT NULL,
    venue         String        NOT NULL,
    base          String        NOT NULL,
    quote         String        NOT NULL,
    market_type   String        NOT NULL,
    bid_price     Decimal(38,18) NOT NULL,
    bid_size      Decimal(38,18) NOT NULL,
    ask_price     Decimal(38,18) NOT NULL,
    ask_size      Decimal(38,18) NOT NULL
)
ENGINE = MergeTree()
ORDER BY (venue, base, quote, market_type, received_at)
PARTITION BY toYYYYMMDD(received_at)
TTL received_at + INTERVAL 30 DAY
SETTINGS index_granularity = 8192;
