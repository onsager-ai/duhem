//! `db/query` — read rows from a real SQL database for assertions.
//!
//! `with:` shape:
//!
//! - `connection`: full database URL (`postgres://…`, `mysql://…`,
//!   `sqlite:…`). Whole-string template input recommended
//!   (`$inputs.db_url` / `$env.DATABASE_URL`).
//! - `sql`: the query to run. `?` placeholders bind from `params`.
//! - `params` (optional): scalar bind values, in order.
//! - `within` (optional): wall-clock budget for connect + query.
//!
//! Outputs:
//!
//! - `rows`: array of row objects (column name → value). Reach a field
//!   with #104 navigation: `$steps.q.outputs.rows[0].status`.
//! - `row_count`: number of rows returned, as an integer.
//!
//! Outcome: a completed query is `Outcome::Ok` (the rows are data,
//! judged by assertions); `within:` exceeded → `Outcome::Timeout`; a
//! connect / SQL error → `ActionError::Db` → `Outcome::Error`.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::db::{connect, parse_with, row_to_json};
use crate::error::ActionError;
use crate::with::WithinSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct With {
    connection: String,
    sql: String,
    #[serde(default)]
    params: Vec<serde_yml::Value>,
    #[serde(default)]
    within: Option<WithinSpec>,
}

pub struct Query;

#[async_trait]
impl Action for Query {
    fn uses(&self) -> &'static str {
        "db/query"
    }

    async fn invoke(
        &self,
        _ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        execute(parse_with("db/query", with)?).await
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
    let mut conn = connect(&with.connection).await?;

    let mut q = sqlx::query(&with.sql);
    for p in &with.params {
        q = bind_param(q, p)?;
    }
    let rows = q
        .fetch_all(&mut conn)
        .await
        .map_err(|e| ActionError::Db(format!("db/query: {e}")))?;

    let json_rows: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    let row_count = json_rows.len() as i64;

    Ok(ActionResult::ok()
        .with_output("rows", serde_json::Value::Array(json_rows))
        .with_output("row_count", serde_json::Value::from(row_count)))
}

/// Bind one scalar YAML param to the query. Non-scalar params (mapping /
/// sequence) are rejected — bind values must be scalars.
fn bind_param<'q>(
    q: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>,
    p: &serde_yml::Value,
) -> Result<sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>, ActionError> {
    use serde_yml::Value as Y;
    Ok(match p {
        Y::Null => q.bind(None::<String>),
        Y::Bool(b) => q.bind(*b),
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.bind(i)
            } else if let Some(f) = n.as_f64() {
                q.bind(f)
            } else {
                return Err(ActionError::Db(format!("db/query: unbindable number {n}")));
            }
        }
        Y::String(s) => q.bind(s.clone()),
        other => {
            return Err(ActionError::Db(format!(
                "db/query: params must be scalars (got {other:?})"
            )));
        }
    })
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
        let r: Result<With, _> =
            serde_yml::from_str(r#"{ connection: "x", sql: "y", color: red }"#);
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn reads_rows_and_count() {
        // Seed a real in-memory SQLite DB, then read it back. SQLite is
        // a real database engine, not a mock of one.
        let r = execute(parse(&format!(
            r#"
connection: "{MEM}"
sql: "select 7 as id, 'shipped' as status"
"#
        )))
        .await
        .expect("query");
        assert_eq!(r.outputs.get("row_count").and_then(|v| v.as_i64()), Some(1));
        let rows = r.outputs.get("rows").and_then(|v| v.as_array()).unwrap();
        assert_eq!(rows[0]["id"], serde_json::json!(7));
        assert_eq!(rows[0]["status"], serde_json::json!("shipped"));
    }

    #[tokio::test]
    async fn binds_params() {
        let r = execute(parse(&format!(
            r#"
connection: "{MEM}"
sql: "select ? as n"
params: [42]
"#
        )))
        .await
        .expect("query");
        let rows = r.outputs.get("rows").and_then(|v| v.as_array()).unwrap();
        assert_eq!(rows[0]["n"], serde_json::json!(42));
    }

    #[tokio::test]
    async fn empty_result_is_zero_rows() {
        let r = execute(parse(&format!(
            r#"
connection: "{MEM}"
sql: "select 1 as x where 1 = 0"
"#
        )))
        .await
        .expect("query");
        assert_eq!(r.outputs.get("row_count").and_then(|v| v.as_i64()), Some(0));
    }

    #[tokio::test]
    async fn bad_sql_is_db_error() {
        let r = execute(parse(&format!(
            r#"{{ connection: "{MEM}", sql: "select from nope" }}"#
        )))
        .await;
        match r {
            Err(ActionError::Db(_)) => {}
            other => panic!("expected ActionError::Db, got {other:?}"),
        }
    }
}
