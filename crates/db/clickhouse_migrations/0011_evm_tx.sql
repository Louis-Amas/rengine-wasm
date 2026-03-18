CREATE TABLE IF NOT EXISTS evm_tx (
    created_at DateTime64(3, 'UTC'),
    tx_hash String,
    block_number UInt64,
    nonce UInt64,
    from String,
    to String,
    value String,
    gas_limit UInt64,
    gas_price Nullable(String),
    max_fee_per_gas Nullable(String),
    max_priority_fee_per_gas Nullable(String),
    data String,
    transfers Nullable(JSON),
    error Nullable(String)
) ENGINE = MergeTree()
ORDER BY (created_at, tx_hash);
