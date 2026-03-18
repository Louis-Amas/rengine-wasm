use anyhow::Result;
use duckdb::{params, Connection};
use rengine_types::db::TransformerDb;
use std::result::Result as StdResult;
use tokio::sync::oneshot;
use tracing::error;

pub(super) fn handle_add_transformer(
    conn: &Connection,
    transformer: TransformerDb,
    resp: oneshot::Sender<Result<()>>,
) {
    let result = conn.execute(
        "INSERT OR REPLACE INTO transformer (transformer_name, wasm, enabled) VALUES (?1, ?2, ?3);",
        params![
            transformer.transformer_name,
            transformer.wasm,
            transformer.enabled
        ],
    );
    let _ = resp.send(result.map(|_| ()).map_err(Into::into));
}

pub(super) fn handle_toggle_transformer(
    conn: &Connection,
    name: String,
    enabled: bool,
    resp: oneshot::Sender<Result<()>>,
) {
    let result = conn.execute(
        "UPDATE transformer SET enabled = ?1 WHERE transformer_name = ?2;",
        params![enabled, name],
    );
    let _ = resp.send(result.map(|_| ()).map_err(Into::into));
}

pub(super) fn handle_list_transformers(
    conn: &Connection,
    resp: oneshot::Sender<Result<Vec<TransformerDb>>>,
) {
    let mut stmt = match conn.prepare("SELECT transformer_name, wasm, enabled FROM transformer;") {
        Ok(s) => s,
        Err(e) => {
            let _ = resp.send(Err(e.into()));
            return;
        }
    };

    let rows = stmt.query_map([], |row| {
        Ok(TransformerDb {
            transformer_name: row.get(0)?,
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
