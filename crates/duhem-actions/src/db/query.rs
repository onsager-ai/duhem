//! `db/query` — read rows from a real database for assertions.
//!
//! The connection-URL scheme selects the path. SQL URLs (`postgres://`,
//! `mysql://`, `sqlite:`) take a `sql:` query; MongoDB URLs
//! (`mongodb://`, `mongodb+srv://`) take a `find:` block. Both produce
//! the same `rows` / `row_count` outputs, so assertions and #104 nested
//! navigation are identical regardless of backend.
//!
//! `with:` shape:
//!
//! - `connection`: full database URL. Whole-string template input
//!   recommended (`$inputs.db_url` / `$env.DATABASE_URL`). A
//!   `mongodb://` URL must name a default database
//!   (`mongodb://host:port/<db>`).
//! - `sql` (SQL only): the query to run. `?` placeholders bind from
//!   `params`.
//! - `params` (SQL only): scalar bind values, in order.
//! - `find` (MongoDB only): `collection` plus an optional `filter`,
//!   `projection`, `sort`, and `limit`. `filter`/`projection`/`sort`
//!   are BSON documents written as YAML mappings; `filter` defaults to
//!   `{}` (match everything).
//! - `within` (optional): wall-clock budget for connect + query.
//!
//! Outputs:
//!
//! - `rows`: array of row objects (SQL column name → value, or Mongo
//!   document field → value). Reach a field with #104 navigation:
//!   `$steps.q.outputs.rows[0].status`.
//! - `row_count`: number of rows / documents returned, as an integer.
//!
//! BSON values map to judge-comparable JSON scalars: an `ObjectId`
//! becomes its 24-hex string and a `DateTime` becomes an RFC3339
//! string, so `$steps.q.outputs.rows[0]._id` is a plain string.
//!
//! Outcome: a completed query is `Outcome::Ok` (the rows are data,
//! judged by assertions); `within:` exceeded → `Outcome::Timeout`; a
//! connect / query / shape error → `ActionError::Db` → `Outcome::Error`.

use std::time::Duration;

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::Client;
use mongodb::bson::{Bson, Document};
use mongodb::options::ClientOptions;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::db::{connect, parse_with, row_to_json};
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
}

/// MongoDB `find` request: the collection plus optional shaping. The
/// document fields are YAML mappings converted to BSON.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindSpec {
    collection: String,
    #[serde(default)]
    filter: serde_yml::Value,
    #[serde(default)]
    projection: Option<serde_yml::Value>,
    #[serde(default)]
    sort: Option<serde_yml::Value>,
    #[serde(default)]
    limit: Option<i64>,
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
    if is_mongo_url(&with.connection) {
        run_mongo(with).await
    } else {
        run_sql(with).await
    }
}

/// MongoDB connections are routed by URL scheme; everything else is a
/// SQL URL handed to sqlx's `Any` driver.
fn is_mongo_url(url: &str) -> bool {
    url.starts_with("mongodb://") || url.starts_with("mongodb+srv://")
}

async fn run_sql(with: With) -> Result<ActionResult, ActionError> {
    if with.find.is_some() {
        return Err(ActionError::Db(
            "db/query: `find:` is only valid for mongodb:// connections; \
             a SQL connection uses `sql:`"
                .into(),
        ));
    }
    let sql = with.sql.as_deref().ok_or_else(|| {
        ActionError::Db("db/query: a SQL connection requires a `sql:` query".into())
    })?;

    let mut conn = connect(&with.connection).await?;

    let mut q = sqlx::query(sql);
    for p in &with.params {
        q = bind_param(q, p)?;
    }
    let rows = q
        .fetch_all(&mut conn)
        .await
        .map_err(|e| ActionError::Db(format!("db/query: {e}")))?;

    let json_rows: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(rows_result(json_rows))
}

async fn run_mongo(with: With) -> Result<ActionResult, ActionError> {
    if with.sql.is_some() || !with.params.is_empty() {
        return Err(ActionError::Db(
            "db/query: `sql:`/`params:` are only valid for SQL connections; \
             a mongodb:// connection uses `find:`"
                .into(),
        ));
    }
    let find = with.find.ok_or_else(|| {
        ActionError::Db("db/query: a mongodb:// connection requires a `find:` block".into())
    })?;

    let opts = ClientOptions::parse(&with.connection)
        .await
        .map_err(|e| ActionError::Db(format!("db/query: invalid mongodb url: {e}")))?;
    let db_name = opts.default_database.clone().ok_or_else(|| {
        ActionError::Db(
            "db/query: mongodb url must name a database (mongodb://host:port/<db>)".into(),
        )
    })?;
    let client = Client::with_options(opts)
        .map_err(|e| ActionError::Db(format!("db/query: mongodb client: {e}")))?;
    let coll = client
        .database(&db_name)
        .collection::<Document>(&find.collection);

    let filter = to_bson_document("filter", find.filter)?;
    let mut req = coll.find(filter);
    if let Some(p) = find.projection {
        req = req.projection(to_bson_document("projection", p)?);
    }
    if let Some(s) = find.sort {
        req = req.sort(to_bson_document("sort", s)?);
    }
    if let Some(limit) = find.limit {
        req = req.limit(limit);
    }

    let cursor = req
        .await
        .map_err(|e| ActionError::Db(format!("db/query: mongodb find: {e}")))?;
    let docs: Vec<Document> = cursor
        .try_collect()
        .await
        .map_err(|e| ActionError::Db(format!("db/query: mongodb cursor: {e}")))?;

    let json_rows: Vec<serde_json::Value> = docs
        .into_iter()
        .map(|d| bson_to_json(Bson::Document(d)))
        .collect();
    Ok(rows_result(json_rows))
}

/// Pack rows into the shared `rows` / `row_count` output contract.
fn rows_result(json_rows: Vec<serde_json::Value>) -> ActionResult {
    let row_count = json_rows.len() as i64;
    ActionResult::ok()
        .with_output("rows", serde_json::Value::Array(json_rows))
        .with_output("row_count", serde_json::Value::from(row_count))
}

/// Convert a YAML mapping (`filter` / `projection` / `sort`) to a BSON
/// document. A null/absent value is an empty document (match all).
fn to_bson_document(label: &str, v: serde_yml::Value) -> Result<Document, ActionError> {
    if matches!(v, serde_yml::Value::Null) {
        return Ok(Document::new());
    }
    mongodb::bson::serialize_to_document(&v)
        .map_err(|e| ActionError::Db(format!("db/query: `{label}` must be a document: {e}")))
}

/// Render a BSON value as judge-comparable JSON: an `ObjectId` collapses
/// to its 24-hex string and a `DateTime` to RFC3339, so assertions
/// compare plain scalars. Documents/arrays recurse; exotic types
/// (Decimal128, Binary, …) fall back to relaxed extended JSON.
fn bson_to_json(b: Bson) -> serde_json::Value {
    use serde_json::Value as J;
    match b {
        Bson::Double(f) => serde_json::Number::from_f64(f)
            .map(J::Number)
            .unwrap_or(J::Null),
        Bson::String(s) => J::String(s),
        Bson::Boolean(b) => J::Bool(b),
        Bson::Int32(i) => J::from(i),
        Bson::Int64(i) => J::from(i),
        Bson::Null | Bson::Undefined => J::Null,
        Bson::ObjectId(oid) => J::String(oid.to_hex()),
        Bson::DateTime(dt) => dt.try_to_rfc3339_string().map(J::String).unwrap_or(J::Null),
        Bson::Document(d) => J::Object(d.into_iter().map(|(k, v)| (k, bson_to_json(v))).collect()),
        Bson::Array(a) => J::Array(a.into_iter().map(bson_to_json).collect()),
        // Exotic types (Decimal128, Binary, Timestamp, RegEx, …) aren't
        // used by the dogfood assertions; surface a stable string repr
        // rather than depending on bson's extended-JSON feature.
        other => J::String(other.to_string()),
    }
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

    #[test]
    fn parses_find_block() {
        let w = parse(
            r#"
connection: "mongodb://127.0.0.1:27018/crawlab"
find:
  collection: projects
  filter: { name: duhem }
  sort: { _id: -1 }
  limit: 1
"#,
        );
        let find = w.find.expect("find");
        assert_eq!(find.collection, "projects");
        assert_eq!(find.limit, Some(1));
    }

    #[tokio::test]
    async fn find_on_sql_url_is_rejected() {
        let r = execute(parse(&format!(
            r#"
connection: "{MEM}"
find:
  collection: projects
"#
        )))
        .await;
        assert!(matches!(r, Err(ActionError::Db(_))));
    }

    #[tokio::test]
    async fn sql_on_mongo_url_is_rejected() {
        // Validation happens before any connection attempt, so no live
        // mongod is needed to exercise the shape error.
        let r = execute(parse(
            r#"
connection: "mongodb://127.0.0.1:27018/crawlab"
sql: "select 1"
"#,
        ))
        .await;
        assert!(matches!(r, Err(ActionError::Db(_))));
    }

    #[tokio::test]
    async fn mongo_url_without_find_is_rejected() {
        let r = execute(parse(r#"connection: "mongodb://127.0.0.1:27018/crawlab""#)).await;
        assert!(matches!(r, Err(ActionError::Db(_))));
    }

    #[test]
    fn bson_maps_to_judge_comparable_json() {
        use mongodb::bson::{DateTime, doc, oid::ObjectId};

        let oid = ObjectId::parse_str("65f0000000000000000000aa").unwrap();
        let dt = DateTime::parse_rfc3339_str("2026-06-24T12:00:00Z").unwrap();
        let json = bson_to_json(Bson::Document(doc! {
            "_id": oid,
            "name": "duhem",
            "count": 3_i64,
            "created_at": dt,
            "tags": ["a", "b"],
            "nested": { "ok": true },
        }));

        assert_eq!(json["_id"], serde_json::json!("65f0000000000000000000aa"));
        assert_eq!(json["name"], serde_json::json!("duhem"));
        assert_eq!(json["count"], serde_json::json!(3));
        assert_eq!(
            json["created_at"],
            serde_json::json!("2026-06-24T12:00:00Z")
        );
        assert_eq!(json["tags"], serde_json::json!(["a", "b"]));
        assert_eq!(json["nested"]["ok"], serde_json::json!(true));
    }

    /// Live read against a real MongoDB. Ignored by default (no
    /// in-memory Mongo exists, unlike SQLite); run with a reachable
    /// server: `DUHEM_MONGO_TEST_URL=mongodb://127.0.0.1:27018/test \
    /// cargo test -p duhem-actions -- --ignored mongo_reads`.
    #[tokio::test]
    #[ignore = "requires a live mongod via DUHEM_MONGO_TEST_URL"]
    async fn mongo_reads_documents() {
        let url = std::env::var("DUHEM_MONGO_TEST_URL").expect("DUHEM_MONGO_TEST_URL");
        let r = execute(parse(&format!(
            r#"
connection: "{url}"
find:
  collection: duhem_query_probe
  limit: 1
"#
        )))
        .await
        .expect("mongo find");
        // The collection may be empty; the contract is a well-formed
        // result, not a particular row.
        assert!(r.outputs.get("rows").and_then(|v| v.as_array()).is_some());
        assert!(
            r.outputs
                .get("row_count")
                .and_then(|v| v.as_i64())
                .is_some()
        );
    }
}
