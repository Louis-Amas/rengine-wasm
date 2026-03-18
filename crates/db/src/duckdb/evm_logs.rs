use anyhow::Result;
use duckdb::{params, Connection};
use rengine_types::{db::EvmLogsDb, Venue};
use std::result::Result as StdResult;
use tokio::sync::oneshot;
use tracing::error;

pub(super) fn handle_add_evm_logs(
    conn: &Connection,
    evm_logs: EvmLogsDb,
    resp: oneshot::Sender<Result<()>>,
) {
    let result = conn.execute(
        "INSERT OR REPLACE INTO evm_logs (venue, name, wasm) VALUES (?1, ?2, ?3);",
        params![evm_logs.venue, evm_logs.name, evm_logs.wasm],
    );
    let _ = resp.send(result.map(|_| ()).map_err(Into::into));
}

pub(super) fn handle_remove_evm_logs(
    conn: &Connection,
    venue: Venue,
    name: String,
    resp: oneshot::Sender<Result<()>>,
) {
    let result = conn.execute(
        "DELETE FROM evm_logs WHERE venue = ?1 AND name = ?2;",
        params![venue.to_string(), name],
    );
    let _ = resp.send(result.map(|_| ()).map_err(Into::into));
}

pub(super) fn handle_list_evm_logs(
    conn: &Connection,
    venue: Venue,
    resp: oneshot::Sender<Result<Vec<EvmLogsDb>>>,
) {
    let mut stmt = match conn.prepare("SELECT venue, name, wasm FROM evm_logs WHERE venue = ?1;") {
        Ok(s) => s,
        Err(e) => {
            let _ = resp.send(Err(e.into()));
            return;
        }
    };

    let rows = stmt.query_map(params![venue.to_string()], |row| {
        Ok(EvmLogsDb {
            venue: row.get(0)?,
            name: row.get(1)?,
            wasm: row.get(2)?,
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
