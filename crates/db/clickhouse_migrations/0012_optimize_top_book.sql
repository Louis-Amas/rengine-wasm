ALTER TABLE top_book
    MODIFY COLUMN venue LowCardinality(String),
    MODIFY COLUMN base LowCardinality(String),
    MODIFY COLUMN quote LowCardinality(String),
    MODIFY COLUMN market_type LowCardinality(String);
