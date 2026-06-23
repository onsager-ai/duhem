//! `db/seed` — prepare rows in a real SQL database as a precondition.
//!
//! Intended for a verification's `setup:` block: establish the data a
//! check needs against the **real** database before the check runs.
//!
//! `with:` shape:
//!
//! - `connection`: full database URL (see `db/query`).
//! - `sql`: one or more statements (`;`-separated) — DDL and/or inserts.
//!   Run as a raw script, so a seed can `CREATE TABLE IF NOT EXISTS`
//!   then `INSERT` in one step.
//! - `within` (optional): wall-clock budget.
//!
//! Output:
//!
//! - `rows_affected`: total rows affected, as an integer.
//!
//! Outcome: success → `Outcome::Ok`; `within:` exceeded →
//! `Outcome::Timeout`; connect / SQL error → `ActionError::Db` →
//! `Outcome::Error`.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::db::{connect, parse_with};
use crate::error::ActionError;
use crate::with::WithinSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct With {
    connection: String,
    sql: String,
    #[serde(default)]
    within: Option<WithinSpec>,
}

pub struct Seed;

#[async_trait]
impl Action for Seed {
    fn uses(&self) -> &'static str {
        "db/seed"
    }

    async fn invoke(
        &self,
        _ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        execute(parse_with("db/seed", with)?).await
    }
}

pub(crate) async fn execute(with: With) -> Result<ActionResult, ActionError> {
    let timeout: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);
    match tokio::time::timeout(timeout, run(with)).await {
        Ok(result) => result,
        Err(_elapsed) => Ok(ActionResult::timeout()),
    }
}

async fn run(with: With) -> Result<ActionResult, ActionError> {
    use sqlx::Executor;

    let mut conn = connect(&with.connection).await?;

    // `raw_sql` runs a multi-statement script (no binds) — DDL + inserts
    // in one seed. Sum the per-statement affected counts.
    let mut affected: u64 = 0;
    let mut results = conn.execute_many(sqlx::raw_sql(&with.sql));
    use futures::StreamExt;
    while let Some(res) = results.next().await {
        let r = res.map_err(|e| ActionError::Db(format!("db/seed: {e}")))?;
        affected += r.rows_affected();
    }
    drop(results);

    Ok(ActionResult::ok().with_output("rows_affected", serde_json::Value::from(affected as i64)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> With {
        serde_yml::from_value(serde_yml::from_str::<serde_yml::Value>(s).unwrap())
            .expect("With deserialization")
    }

    const MEM: &str = "sqlite::memory:";

    #[test]
    fn rejects_unknown_field() {
        let r: Result<With, _> = serde_yml::from_str(r#"{ connection: "x", sql: "y", z: 1 }"#);
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn seeds_and_reports_rows_affected() {
        // DDL + inserts in one script against a real SQLite engine.
        let r = execute(parse(&format!(
            r#"
connection: "{MEM}"
sql: |
  create table tasks (id integer primary key, status text);
  insert into tasks (id, status) values (1, 'finished'), (2, 'finished');
"#
        )))
        .await
        .expect("seed");
        assert_eq!(
            r.outputs.get("rows_affected").and_then(|v| v.as_i64()),
            Some(2)
        );
    }

    #[tokio::test]
    async fn bad_sql_is_db_error() {
        let r = execute(parse(&format!(
            r#"{{ connection: "{MEM}", sql: "create nonsense" }}"#
        )))
        .await;
        match r {
            Err(ActionError::Db(_)) => {}
            other => panic!("expected ActionError::Db, got {other:?}"),
        }
    }
}
