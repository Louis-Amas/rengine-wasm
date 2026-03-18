use crate::duckdb::MultiCallId;
use anyhow::Result;
use duckdb::{params, Connection};
use rengine_types::{db::MultiCallDb, Venue};
use std::result::Result as StdResult;
use tokio::sync::oneshot;
use tracing::error;

pub(super) fn handle_add_multicall(
    conn: &Connection,
    multicall: MultiCallDb,
    resp: oneshot::Sender<Result<()>>,
) {
    let result = conn.execute(
        "INSERT OR REPLACE INTO multicall (venue, name, wasm) VALUES (?1, ?2, ?3);",
        params![multicall.venue, multicall.name, multicall.wasm],
    );
    let _ = resp.send(result.map(|_| ()).map_err(Into::into));
}

pub(super) fn handle_remove_multicall(
    conn: &Connection,
    venue: Venue,
    name: MultiCallId,
    resp: oneshot::Sender<Result<()>>,
) {
    let result = conn.execute(
        "DELETE FROM multicall WHERE venue = ?1 AND name = ?2;",
        params![venue.to_string(), name.to_string()],
    );
    let _ = resp.send(result.map(|_| ()).map_err(Into::into));
}

pub(super) fn handle_list_multicalls(
    conn: &Connection,
    venue: Venue,
    resp: oneshot::Sender<Result<Vec<MultiCallDb>>>,
) {
    let mut stmt = match conn.prepare("SELECT venue, name, wasm FROM multicall WHERE venue = ?1;") {
        Ok(s) => s,
        Err(e) => {
            let _ = resp.send(Err(e.into()));
            return;
        }
    };

    let rows = stmt.query_map(params![venue.to_string()], |row| {
        Ok(MultiCallDb {
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
