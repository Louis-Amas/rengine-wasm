use super::utils::{value_to_datetime, value_to_decimal};
use anyhow::{anyhow, Result};
use chrono::Utc;
use duckdb::{params, Connection};
use rengine_types::{
    db::{Exposure, TradeDb},
    Account,
};
use std::result::Result as StdResult;
use tokio::sync::oneshot;
use tracing::error;

pub(super) fn handle_add_trades(
    conn: &mut Connection,
    trades: Vec<TradeDb>,
    resp: oneshot::Sender<Result<()>>,
) {
    let tx = match conn.transaction() {
        Ok(tx) => tx,
        Err(err) => {
            error!("catastrohic failure {err:?}");
            return;
        }
    };

    let mut error: Option<anyhow::Error> = None;
    for trade in trades {
        let _ = match tx.execute(
            "INSERT INTO trade (
                    received_at,
                    emitted_at,
                    order_id,
                    trade_id,
                    account,
                    base,
                    quote,
                    side,
                    market_type,
                    price,
                    size,
                    fee,
                    fee_symbol
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13);",
            params![
                trade.received_at.to_string(),
                trade.emitted_at.to_string(),
                trade.order_id.to_string(),
                trade.trade_id.to_string(),
                trade.account,
                trade.base.to_string(),
                trade.quote.to_string(),
                trade.side,
                trade.market_type,
                trade.price.to_string(),
                trade.size.to_string(),
                trade.fee.to_string(),
                trade.fee_symbol.to_string(),
            ],
        ) {
            Ok(result) => result,
            Err(err) => {
                error = Some(anyhow!("couldn't insert trade {err:?}"));
                break;
            }
        };
    }

    if let Some(error) = error {
        if let Err(err) = tx.rollback() {
            error!("couldn't rollback rx {err:?}");
        }
        let _ = resp.send(Err(error));
    } else {
        // commit transaction if all succeeded
        let commit_res = tx.commit();
        let _ = resp.send(commit_res.map(|_| ()).map_err(Into::into));
    }
}

pub(super) fn handle_list_trades(
    conn: &Connection,
    account: Account,
    resp: oneshot::Sender<Result<Vec<TradeDb>>>,
) {
    let mut stmt = match conn.prepare(
        "SELECT
                received_at,
                emitted_at,
                order_id,
                trade_id,
                account,
                base,
                quote,
                side,
                market_type,
                price,
                size,
                fee,
                fee_symbol
            FROM trade
            WHERE account = ?1
        ORDER BY received_at DESC;",
    ) {
        Ok(s) => s,
        Err(e) => {
            let _ = resp.send(Err(e.into()));
            return;
        }
    };

    let rows = stmt.query_map(params![account.to_string()], |row| {
        Ok(TradeDb {
            received_at: value_to_datetime(row.get(0)?)?,
            emitted_at: value_to_datetime(row.get(1)?)?,
            order_id: row.get::<_, i64>(2)?,
            trade_id: row.get::<_, i64>(3)?,
            account: row.get(4)?,
            base: row.get::<_, String>(5)?.into(),
            quote: row.get::<_, String>(6)?.into(),
            side: row.get(7)?,
            market_type: row.get(8)?,
            price: value_to_decimal(row.get(9)?)?,
            size: value_to_decimal(row.get(10)?)?,
            fee: value_to_decimal(row.get(11)?)?,
            fee_symbol: row.get::<_, String>(12)?.into(),
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
                error!("error when reading trades {err:?}");
            }
        }
        Err(e) => {
            let _ = resp.send(Err(e.into()));
        }
    }
}

pub(super) fn handle_load_exposures(
    conn: &Connection,
    account: Account,
    resp: oneshot::Sender<Result<Vec<Exposure>>>,
) {
    const EXPOSURE_QUERY: &str = r#"
    SELECT
        account,
        base,
        quote,
        CAST(
            SUM(
                CASE WHEN side = 'bid' THEN CAST(size AS DECIMAL(28,18))
                     WHEN side = 'ask' THEN -CAST(size AS DECIMAL(28,18))
                     ELSE CAST(0 AS DECIMAL(28,18)) END
            ) AS DECIMAL(28,18)
        ) AS base_exposure,
        CAST(
            SUM(
                CASE WHEN side = 'bid' THEN -(
                    CAST(size AS DECIMAL(28,18)) * CAST(price AS DECIMAL(28,10))
                )
                WHEN side = 'ask' THEN (
                    CAST(size AS DECIMAL(28,18)) * CAST(price AS DECIMAL(28,10))
                )
                ELSE CAST(0 AS DECIMAL(28,18)) END
            ) AS DECIMAL(28,18)
        ) AS quote_exposure,
        MAX(emitted_at) as latest_emitted,
    FROM trade
    WHERE account = ?1
    GROUP BY account, base, quote
    ORDER BY account, base, quote
"#;

    let mut stmt = match conn.prepare(EXPOSURE_QUERY) {
        Ok(s) => s,
        Err(e) => {
            let _ = resp.send(Err(e.into()));
            return;
        }
    };

    let rows = stmt.query_map([account.to_string()], |row| {
        Ok(Exposure {
            account: row.get::<_, String>(0)?.parse().unwrap(),
            base: row.get::<_, String>(1)?.into(),
            quote: row.get::<_, String>(2)?.into(),
            base_exposure: value_to_decimal(row.get(3)?)?,
            quote_exposure: value_to_decimal(row.get(4)?)?,
            at: Utc::now().into(),
            latest_emitted_at: value_to_datetime(row.get(5)?)?.into(),
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
                error!("error when reading exposures {err:?}");
            }
        }
        Err(e) => {
            let _ = resp.send(Err(e.into()));
        }
    }
}
