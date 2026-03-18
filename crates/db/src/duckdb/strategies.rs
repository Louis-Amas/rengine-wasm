use anyhow::Result;
use duckdb::{params, Connection};
use rengine_types::db::StrategyDb;
use std::result::Result as StdResult;
use tokio::sync::oneshot;
use tracing::error;

pub(super) fn handle_add_strategy(
    conn: &Connection,
    strategy: StrategyDb,
    resp: oneshot::Sender<Result<()>>,
) {
    let result = conn.execute(
        "INSERT OR REPLACE INTO strategy (strategy_name, wasm, enabled) VALUES (?1, ?2, ?3);",
        params![strategy.strategy_name, strategy.wasm, strategy.enabled],
    );
    let _ = resp.send(result.map(|_| ()).map_err(Into::into));
}

pub(super) fn handle_toggle_strategy(
    conn: &Connection,
    name: String,
    enabled: bool,
    resp: oneshot::Sender<Result<()>>,
) {
    let result = conn.execute(
        "UPDATE strategy SET enabled = ?1 WHERE strategy_name = ?2;",
        params![enabled, name],
    );
    let _ = resp.send(result.map(|_| ()).map_err(Into::into));
}

pub(super) fn handle_list_strategies(
    conn: &Connection,
    resp: oneshot::Sender<Result<Vec<StrategyDb>>>,
) {
    let mut stmt = match conn.prepare("SELECT strategy_name, wasm, enabled FROM strategy;") {
        Ok(s) => s,
        Err(e) => {
            let _ = resp.send(Err(e.into()));
            return;
        }
    };

    let rows = stmt.query_map([], |row| {
        Ok(StrategyDb {
            strategy_name: row.get(0)?,
            wasm: row.get(1)?,
            enabled: row.get(2)?,
        })
    });

    match rows {
        Ok(mapped) => {
            let result: StdResult<Vec<_>, duckdb::Error> = mapped.collect();
            let result = match result {
                Ok(result) => resp.send(Ok(result)),
                Err(err) => resp.send(Err(err.into())),
            };

            if let Err(err) = result {
                error!("error when reading rows {err:?}");
            }
        }
        Err(e) => {
            let _ = resp.send(Err(e.into()));
        }
    }
}
