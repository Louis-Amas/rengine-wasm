ALTER TABLE balance
    MODIFY COLUMN account LowCardinality(String),
    MODIFY COLUMN symbol LowCardinality(String);
