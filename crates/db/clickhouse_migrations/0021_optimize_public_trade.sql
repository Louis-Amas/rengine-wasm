ALTER TABLE public_trade
    MODIFY COLUMN venue LowCardinality(String),
    MODIFY COLUMN instrument LowCardinality(String),
    MODIFY COLUMN side LowCardinality(String);
