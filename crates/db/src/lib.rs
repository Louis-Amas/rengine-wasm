mod clickhouse;
mod duckdb;
mod migrate;

use crate::{clickhouse::ClickHouseDb, duckdb::DuckDb};
use anyhow::Result;
use rengine_config::DbConfig;
use std::sync::Arc;

pub struct Db {
    pub duckdb: Arc<DuckDb>,
    pub clickhouse: Option<Arc<ClickHouseDb>>,
}

impl Db {
    pub async fn new(config: DbConfig) -> Result<Self> {
        let clickhouse = if let Some(clickhouse_config) = config.clickhouse {
            Some(Arc::new(ClickHouseDb::try_new(clickhouse_config).await?))
        } else {
            None
        };

        Ok(Self {
            duckdb: Arc::new(DuckDb::new(&config.duck_db_path).await?),
            clickhouse,
        })
    }

    pub async fn stop(&self) {
        self.duckdb.stop();
        if let Some(db) = self.clickhouse.as_ref() {
            db.stop().await;
        }
    }
}
