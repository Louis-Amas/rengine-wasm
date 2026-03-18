ALTER TABLE position
    MODIFY COLUMN venue LowCardinality(String),
    MODIFY COLUMN symbol LowCardinality(String),
    MODIFY COLUMN side LowCardinality(String),
    MODIFY COLUMN account_id LowCardinality(String),
    MODIFY COLUMN position_type LowCardinality(String);
