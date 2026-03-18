CREATE TABLE IF NOT EXISTS public_trade (
    received_at DateTime64(9, 'UTC'),
    venue String,
    instrument String,
    price Decimal(38,18),
    size Decimal(38,18),
    side String,
    time DateTime64(9, 'UTC'),
    trade_id String
)
ENGINE = MergeTree()
ORDER BY (venue, instrument, received_at)
PARTITION BY toYYYYMMDD(received_at)
TTL received_at + INTERVAL 30 DAY;

