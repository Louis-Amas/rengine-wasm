ALTER TABLE trade
    MODIFY COLUMN account LowCardinality(String),
    MODIFY COLUMN base LowCardinality(String),
    MODIFY COLUMN quote LowCardinality(String),
    MODIFY COLUMN side LowCardinality(String),
    MODIFY COLUMN market_type LowCardinality(String),
    MODIFY COLUMN fee_symbol LowCardinality(String);
