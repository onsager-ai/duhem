//! `db/observe` — read a database on an interval until the returned rows
//! satisfy a condition, or a timeout elapses. The DB analogue of
//! `api/poll` (#115): where [`Query`](super::Query) reads once and
//! assertions evaluate that single snapshot, `db/observe` reads until
//! `until` holds, so a check can read-*after-settle* against an
//! eventually-consistent backend without a flaky fixed `sleep`.
//!
//! Motivated by #179: crawlab-pro's async master↔worker spider sync
//! transiently `ReplaceOne`s a spider doc from a partially-populated
//! struct, so a one-shot `db/query` read-back after a PATCH can catch
//! the row mid-flight (`name:""`) and false-fail. `db/observe` polls the
//! `_id` doc until `name` settles, and — crucially — distinguishes a
//! transient race (settles → `satisfied: true`) from a persistent defect
//! (never settles → times out, `satisfied: false`, the check stays red).
//!
//! `with:` mirrors `db/query` (`connection` + `find:` / `sql:`+`params:`)
//! plus `api/poll`'s loop knobs:
//!
//! - `within`: total budget (default [`DEFAULT_WITHIN`]).
//! - `interval`: poll cadence (default 1s).
//! - `until`: the stop condition — exactly one mode:
//!     - `{ row_count: <int> }` — poll until exactly N rows are returned.
//!     - `{ path: <path>, equals|matches|exists|gte: … }` — poll until a
//!       field under the result satisfies the predicate. `path` navigates
//!       a synthetic root `{rows: [...], row_count: N}`, so authors write
//!       `rows[0].name` or `row_count` (dotted/bracket path, mirroring
//!       #104 and `api/poll`).
//!
//! Outputs mirror `db/query` plus `satisfied`:
//!
//! - `satisfied`: `true` if `until` held before the budget elapsed.
//! - `rows`: final rows snapshot (array of row objects).
//! - `row_count`: number of rows in the final snapshot.
//!
//! Outcome mirrors `api/poll` / `ui/assert-*`: a completed observe is
//! `Outcome::Ok` with `satisfied` true/false — the verdict stays in the
//! judge (`assertions: - $steps.o.outputs.satisfied == true`); `until` is
//! only the loop's stop-condition. The **first** read is strict: a config
//! error (`find:` on a SQL url, a malformed filter) or an initial connect
//! failure surfaces loudly rather than silently polling to a false
//! timeout. Subsequent reads tolerate a transient blip (a momentary lock
//! contention) as "not yet".

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::api::poll::{navigate, value_as_str, yml_to_json_value};
use crate::db::query::{FindSpec, fetch_rows};
use crate::error::ActionError;
use crate::with::WithinSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct With {
    connection: String,
    #[serde(default)]
    sql: Option<String>,
    #[serde(default)]
    params: Vec<serde_yml::Value>,
    #[serde(default)]
    find: Option<FindSpec>,
    #[serde(default)]
    within: Option<WithinSpec>,
    #[serde(default)]
    interval: Option<WithinSpec>,
    until: Until,
}

/// The poll stop-condition. Exactly one mode must be set: `row_count`, or
/// a `path` predicate (`equals` / `matches` / `exists` / `gte`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Until {
    #[serde(default)]
    row_count: Option<i64>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    equals: Option<serde_yml::Value>,
    #[serde(default)]
    matches: Option<String>,
    #[serde(default)]
    exists: Option<bool>,
    #[serde(default)]
    gte: Option<f64>,
}

pub struct Observe;

#[async_trait]
impl Action for Observe {
    fn uses(&self) -> &'static str {
        "db/observe"
    }

    async fn invoke(
        &self,
        _ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "db/observe",
                source: e,
            })?;
        execute(with).await
    }
}

pub(crate) async fn execute(with: With) -> Result<ActionResult, ActionError> {
    let total: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);
    let interval: Duration = with
        .interval
        .map(Into::into)
        .unwrap_or(Duration::from_secs(1));
    // Compile (and validate) `until` up front so a malformed condition is
    // a loud error, not a silent false timeout.
    let mode = with.compile_until()?;

    let started = Instant::now();

    // First read is strict — surfaces config / initial-connect errors.
    let mut last = fetch(&with).await?;
    if mode.evaluate(&last) {
        return Ok(result(true, Some(last)));
    }

    loop {
        if started.elapsed() >= total {
            return Ok(result(false, Some(last)));
        }
        tokio::time::sleep(interval).await;
        // A transient DB error (a momentary lock, a blip) counts as "not
        // yet": keep the last good snapshot and keep polling.
        if let Ok(rows) = fetch(&with).await {
            if mode.evaluate(&rows) {
                return Ok(result(true, Some(rows)));
            }
            last = rows;
        }
    }
}

async fn fetch(with: &With) -> Result<Vec<serde_json::Value>, ActionError> {
    fetch_rows(
        "db/observe",
        &with.connection,
        with.sql.as_deref(),
        &with.params,
        with.find.as_ref(),
    )
    .await
}

fn result(satisfied: bool, last: Option<Vec<serde_json::Value>>) -> ActionResult {
    let mut r = ActionResult::ok().with_output("satisfied", serde_json::Value::Bool(satisfied));
    if let Some(rows) = last {
        let row_count = rows.len() as i64;
        r = r
            .with_output("rows", serde_json::Value::Array(rows))
            .with_output("row_count", serde_json::Value::from(row_count));
    }
    r
}

/// A validated, ready-to-evaluate stop condition.
enum Mode {
    RowCount(i64),
    Path { path: String, pred: PathPred },
}

enum PathPred {
    Equals(serde_json::Value),
    Matches(regex::Regex),
    Exists(bool),
    Gte(f64),
}

impl Mode {
    fn evaluate(&self, rows: &[serde_json::Value]) -> bool {
        match self {
            Mode::RowCount(want) => rows.len() as i64 == *want,
            Mode::Path { path, pred } => {
                // Navigate a synthetic root so `rows[0].name` and
                // `row_count` both reach real values — the same output
                // shape `db/query` exposes.
                let root = serde_json::json!({
                    "rows": rows,
                    "row_count": rows.len() as i64,
                });
                let found = navigate(&root, path);
                match pred {
                    PathPred::Exists(want) => found.is_some() == *want,
                    PathPred::Equals(want) => found.map(|v| v == want).unwrap_or(false),
                    PathPred::Matches(re) => found
                        .and_then(value_as_str)
                        .map(|s| re.is_match(&s))
                        .unwrap_or(false),
                    PathPred::Gte(n) => found
                        .and_then(|v| v.as_f64())
                        .map(|f| f >= *n)
                        .unwrap_or(false),
                }
            }
        }
    }
}

impl With {
    /// Validate `until` names exactly one mode and compile it.
    fn compile_until(&self) -> Result<Mode, ActionError> {
        let u = &self.until;
        if let Some(n) = u.row_count {
            ensure_no_path_fields(u)?;
            return Ok(Mode::RowCount(n));
        }
        let path = u.path.clone().ok_or_else(|| {
            ActionError::Db(
                "db/observe: `until` must set `row_count:` or a `path:` predicate".to_string(),
            )
        })?;
        let pred = match (&u.equals, &u.matches, u.exists, u.gte) {
            (Some(v), None, None, None) => PathPred::Equals(yml_to_json_value(v)),
            (None, Some(re), None, None) => {
                PathPred::Matches(regex::Regex::new(re).map_err(|e| {
                    ActionError::Db(format!("db/observe: bad `matches` regex: {e}"))
                })?)
            }
            (None, None, Some(b), None) => PathPred::Exists(b),
            (None, None, None, Some(n)) => PathPred::Gte(n),
            _ => {
                return Err(ActionError::Db(
                    "db/observe: a `path` predicate needs exactly one of \
                     equals/matches/exists/gte"
                        .to_string(),
                ));
            }
        };
        Ok(Mode::Path { path, pred })
    }
}

fn ensure_no_path_fields(u: &Until) -> Result<(), ActionError> {
    if u.path.is_some()
        || u.equals.is_some()
        || u.matches.is_some()
        || u.exists.is_some()
        || u.gte.is_some()
    {
        return Err(ActionError::Db(
            "db/observe: `until` mixes `row_count:` with a `path:` predicate; set exactly one"
                .to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn parse(s: &str) -> With {
        serde_yml::from_value(serde_yml::from_str::<serde_yml::Value>(s).unwrap())
            .expect("With deserialization")
    }

    const MEM: &str = "sqlite::memory:";

    /// A unique on-disk SQLite path — an in-memory DB is per-connection,
    /// so it can't share state across the poll's per-iteration connects.
    fn temp_db_url() -> (std::path::PathBuf, String) {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("duhem_observe_{}_{}.db", std::process::id(), n));
        let _ = std::fs::remove_file(&path);
        let url = format!("sqlite://{}?mode=rwc", path.display());
        (path, url)
    }

    async fn exec_sql(url: &str, sql: &str) {
        use sqlx::Executor;
        let mut conn = crate::db::connect(url).await.expect("connect");
        conn.execute(sql).await.expect("ddl/dml");
    }

    #[test]
    fn rejects_unknown_field() {
        let r: Result<With, _> =
            serde_yml::from_str(r#"{ connection: "x", sql: "y", until: { row_count: 1 }, x: 1 }"#);
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn row_count_satisfied_on_first_read() {
        let r = execute(parse(&format!(
            r#"
connection: "{MEM}"
sql: "select 7 as id"
until: {{ row_count: 1 }}
"#
        )))
        .await
        .expect("observe");
        assert_eq!(r.outputs.get("satisfied"), Some(&serde_json::json!(true)));
        assert_eq!(r.outputs.get("row_count").and_then(|v| v.as_i64()), Some(1));
    }

    #[tokio::test]
    async fn path_predicate_satisfied_on_first_read() {
        let r = execute(parse(&format!(
            r#"
connection: "{MEM}"
sql: "select 'shipped' as status"
until: {{ path: "rows[0].status", equals: shipped }}
"#
        )))
        .await
        .expect("observe");
        assert_eq!(r.outputs.get("satisfied"), Some(&serde_json::json!(true)));
    }

    #[tokio::test]
    async fn never_settles_returns_false_within_budget() {
        let r = execute(parse(&format!(
            r#"
connection: "{MEM}"
sql: "select 1 as x where 1 = 0"
within: "1s"
interval: "200ms"
until: {{ row_count: 1 }}
"#
        )))
        .await
        .expect("observe");
        assert_eq!(r.outputs.get("satisfied"), Some(&serde_json::json!(false)));
        assert_eq!(r.outputs.get("row_count").and_then(|v| v.as_i64()), Some(0));
    }

    #[tokio::test]
    async fn settles_after_a_delayed_write() {
        let (path, url) = temp_db_url();
        exec_sql(&url, "create table t (name text)").await;

        // A concurrent writer inserts the row mid-poll — the real
        // read-after-settle shape (a separate connection sharing the
        // on-disk DB, like crawlab's sync writing to Mongo).
        let writer_url = url.clone();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            exec_sql(&writer_url, "insert into t (name) values ('settled')").await;
        });

        let r = execute(parse(&format!(
            r#"
connection: "{url}"
sql: "select name from t"
within: "10s"
interval: "150ms"
until: {{ path: "rows[0].name", equals: settled }}
"#
        )))
        .await
        .expect("observe");
        writer.await.unwrap();

        assert_eq!(r.outputs.get("satisfied"), Some(&serde_json::json!(true)));
        let rows = r.outputs.get("rows").and_then(|v| v.as_array()).unwrap();
        assert_eq!(rows[0]["name"], serde_json::json!("settled"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn config_error_surfaces_on_first_read() {
        // `find:` on a SQL url is a config error — the strict first read
        // returns it rather than polling to a false timeout.
        let r = execute(parse(&format!(
            r#"
connection: "{MEM}"
find:
  collection: t
until: {{ row_count: 1 }}
"#
        )))
        .await;
        assert!(matches!(r, Err(ActionError::Db(_))));
    }

    #[test]
    fn until_mixing_modes_is_rejected() {
        let w = parse(&format!(
            r#"
connection: "{MEM}"
sql: "select 1"
until: {{ row_count: 1, path: "row_count", gte: 1 }}
"#
        ));
        assert!(w.compile_until().is_err());
    }

    #[test]
    fn until_path_needs_exactly_one_predicate() {
        let w = parse(&format!(
            r#"
connection: "{MEM}"
sql: "select 1"
until: {{ path: "rows[0].name", equals: a, exists: true }}
"#
        ));
        assert!(w.compile_until().is_err());
    }
}
