//! `db/*` actions — read and seed a **real** database.
//!
//! Two actions ship: [`Query`] (`db/query`, read rows for assertions)
//! and [`Seed`] (`db/seed`, prepare rows as a precondition). The SQL
//! path runs against the real database over `sqlx`'s multi-backend
//! `Any` driver — Postgres, MySQL, and SQLite are selected by the
//! connection URL scheme (`postgres://`, `mysql://`, `sqlite:`).
//! `db/query` additionally reads MongoDB (`mongodb://`,
//! `mongodb+srv://`) via a `find:` block, sharing the same `rows` /
//! `row_count` output contract (see [`query`]); `db/seed` is SQL-only.
//! No stub, no in-memory double of the application's store
//! (`docs/duhem-spec.md` §8): the whole point is to confirm what
//! actually landed in the database.
//!
//! These exist for the Crawlab dogfood (#99/#101) — distributed task
//! lifecycle and multi-DB ORM correctness live in database state the
//! `ui/*` + `api/*` catalog can't see.
//!
//! Connection: `connection:` is a full database URL. Authors pass it
//! whole — `$inputs.db_url`, `$env.DATABASE_URL`, or a literal — since
//! template substitution is whole-string. A named-`environments:`
//! connection registry is a separate spec (#68).
//!
//! Value mapping: each result row becomes a JSON object keyed by column
//! name. Column values are decoded by trying, in order, `i64`, `f64`,
//! `bool`, then `String`; a SQL `NULL` is JSON `null`; a type outside
//! that set (timestamp, numeric, uuid, json) currently decodes to
//! `null` (widening the decode set is a follow-up). With #104's nested
//! navigation, an assertion reaches a field as
//! `$steps.q.outputs.rows[0].status`.

use sqlx::{Column, Row, any::AnyRow};

use crate::error::ActionError;

pub mod observe;
pub mod query;
pub mod seed;

pub use observe::Observe as DbObserve;
pub use query::Query;
pub use seed::Seed;

/// Connect to the database named by `url`, dispatching backend by URL
/// scheme. Installs sqlx's default drivers once (idempotent).
pub(crate) async fn connect(url: &str) -> Result<sqlx::AnyConnection, ActionError> {
    use sqlx::ConnectOptions;
    use std::str::FromStr;

    sqlx::any::install_default_drivers();
    let opts = sqlx::any::AnyConnectOptions::from_str(url)
        .map_err(|e| ActionError::Db(format!("db: invalid connection url: {e}")))?;
    opts.connect()
        .await
        .map_err(|e| ActionError::Db(format!("db: connect failed: {e}")))
}

/// Render one result row as a JSON object keyed by column name.
pub(crate) fn row_to_json(row: &AnyRow) -> serde_json::Value {
    let mut obj = serde_json::Map::with_capacity(row.columns().len());
    for col in row.columns() {
        obj.insert(col.name().to_string(), decode_column(row, col.ordinal()));
    }
    serde_json::Value::Object(obj)
}

/// Decode one column to JSON by trying the `Any`-supported scalar types
/// in turn. `try_get` returns `Err` on a type mismatch (try the next)
/// and `Ok(None)` on SQL `NULL` (→ JSON `null`).
fn decode_column(row: &AnyRow, i: usize) -> serde_json::Value {
    use serde_json::Value as J;
    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
        return v.map(J::from).unwrap_or(J::Null);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
        return v.map(J::from).unwrap_or(J::Null);
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
        return v.map(J::from).unwrap_or(J::Null);
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return v.map(J::from).unwrap_or(J::Null);
    }
    // A column type outside the Any scalar set (timestamp, numeric,
    // uuid, json). Surface null rather than failing the whole row;
    // widening the decode set is a follow-up.
    J::Null
}

/// Shared `Action::invoke` body: deserialize `with` into the action's
/// typed struct, surfacing a uniform `InvalidWith`.
pub(crate) fn parse_with<T: serde::de::DeserializeOwned>(
    action: &'static str,
    with: &serde_yml::Value,
) -> Result<T, ActionError> {
    serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith { action, source: e })
}
