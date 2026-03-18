ALTER TABLE exposure
    MODIFY COLUMN account LowCardinality(String),
    MODIFY COLUMN symbol LowCardinality(String);
