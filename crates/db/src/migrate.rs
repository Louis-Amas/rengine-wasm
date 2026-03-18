use anyhow::{Context, Result};
use std::{borrow::Cow, sync::Arc};
use tracing::{error, info, trace};

/// Handles embedded `ClickHouse` migrations.
/// Differences from file-based version:
/// - Migrations are statically embedded using `include_str!`.
/// - Order is deterministic via the `MIGRATIONS` array.
/// - Each migration is only applied once (tracked in the `migration` table).
pub(crate) struct Migration {
    client: Arc<clickhouse::Client>,
    table_name: Cow<'static, str>,
}

/// Table name used for migration tracking.
const TABLE_NAME: &str = "migration";

/// List of embedded migrations, ordered by filename.
static MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_top_book.sql",
        include_str!("../clickhouse_migrations/0001_top_book.sql"),
    ),
    (
        "0002_balance.sql",
        include_str!("../clickhouse_migrations/0002_balance.sql"),
    ),
    (
        "0003_trade.sql",
        include_str!("../clickhouse_migrations/0003_trade.sql"),
    ),
    (
        "0004_position.sql",
        include_str!("../clickhouse_migrations/0004_position.sql"),
    ),
    (
        "0005_exposure.sql",
        include_str!("../clickhouse_migrations/0005_exposure.sql"),
    ),
    (
        "0006_indicator.sql",
        include_str!("../clickhouse_migrations/0006_indicator.sql"),
    ),
    (
        "0007_strategy_log.sql",
        include_str!("../clickhouse_migrations/0007_strategy_log.sql"),
    ),
    (
        "0008_latency.sql",
        include_str!("../clickhouse_migrations/0008_latency.sql"),
    ),
    (
        "0009_transformer_log.sql",
        include_str!("../clickhouse_migrations/0009_transformer_log.sql"),
    ),
    (
        "0010_public_trade.sql",
        include_str!("../clickhouse_migrations/0010_public_trade.sql"),
    ),
    (
        "0011_evm_tx.sql",
        include_str!("../clickhouse_migrations/0011_evm_tx.sql"),
    ),
    (
        "0012_optimize_top_book.sql",
        include_str!("../clickhouse_migrations/0012_optimize_top_book.sql"),
    ),
    (
        "0013_optimize_balance.sql",
        include_str!("../clickhouse_migrations/0013_optimize_balance.sql"),
    ),
    (
        "0014_optimize_trade.sql",
        include_str!("../clickhouse_migrations/0014_optimize_trade.sql"),
    ),
    (
        "0015_optimize_position.sql",
        include_str!("../clickhouse_migrations/0015_optimize_position.sql"),
    ),
    (
        "0016_optimize_exposure.sql",
        include_str!("../clickhouse_migrations/0016_optimize_exposure.sql"),
    ),
    (
        "0017_optimize_indicator.sql",
        include_str!("../clickhouse_migrations/0017_optimize_indicator.sql"),
    ),
    (
        "0018_optimize_strategy_log.sql",
        include_str!("../clickhouse_migrations/0018_optimize_strategy_log.sql"),
    ),
    (
        "0019_optimize_latency.sql",
        include_str!("../clickhouse_migrations/0019_optimize_latency.sql"),
    ),
    (
        "0020_optimize_transformer_log.sql",
        include_str!("../clickhouse_migrations/0020_optimize_transformer_log.sql"),
    ),
    (
        "0021_optimize_public_trade.sql",
        include_str!("../clickhouse_migrations/0021_optimize_public_trade.sql"),
    ),
    (
        "0022_counter_metric.sql",
        include_str!("../clickhouse_migrations/0022_counter_metric.sql"),
    ),
    (
        "0023_storage.sql",
        include_str!("../clickhouse_migrations/0023_storage.sql"),
    ),
];

impl Migration {
    pub(crate) const fn new(client: Arc<clickhouse::Client>) -> Self {
        Self {
            client,
            table_name: Cow::Borrowed(TABLE_NAME),
        }
    }

    /// Run all embedded migrations that haven't yet been applied.
    pub(crate) async fn run(&self) -> Result<()> {
        info!("Running embedded ClickHouse migrations");
        self.try_init_migration_table().await?;

        let applied = self.retrieve_applied_migrations().await?;

        for (name, sql_query) in MIGRATIONS {
            if applied.contains(&name.to_string()) {
                trace!(?name, "Skipping already applied migration");
                continue;
            }

            info!(?name, "Applying migration");
            if let Err(err) = self.client.query(sql_query).execute().await {
                error!(?err, ?name, "Failed to apply migration");
                return Err(err).with_context(|| format!("Failed to apply migration {name}"));
            }

            // Insert record into migration tracking table
            let insert_query = format!("INSERT INTO {} (version) VALUES (?)", self.table_name);
            self.client
                .query(&insert_query)
                .bind(*name)
                .execute()
                .await
                .with_context(|| format!("Failed to record migration {name}"))?;
        }
        info!("Migrations finished");

        Ok(())
    }

    /// Create the migration tracking table if it doesn't exist.
    async fn try_init_migration_table(&self) -> Result<()> {
        let query = self.client.query(
            r#"CREATE TABLE IF NOT EXISTS migration
                    (
                        version String,
                        applied_at DateTime DEFAULT now()
                    )
                    ENGINE = MergeTree
                    ORDER BY applied_at"#,
        );

        query.execute().await?;
        Ok(())
    }

    /// Retrieve all migration names that have already been applied.
    async fn retrieve_applied_migrations(&self) -> Result<Vec<String>> {
        let query = format!("SELECT version FROM {}", self.table_name);
        let res = self
            .client
            .query(&query)
            .fetch_all::<String>()
            .await
            .context("Failed to fetch applied migrations")?;
        Ok(res)
    }
}
