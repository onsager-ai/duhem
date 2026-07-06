//! `SqliteStore` — the local `Store` implementation (#189).
//!
//! One SQLite database per working copy (see `location.rs`), WAL
//! mode, sqlx migrations applied on writable open. The dashboard uses
//! [`SqliteStore::open_read_only`], which maps to SQLite's `mode=ro`
//! — the read-only-lens invariant is enforced by the connection, not
//! by discipline. Append-only is enforced by triggers in the
//! migration, not here.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Pool, Row, Sqlite};

use crate::event::{Event, EventPayload};
use crate::writer::Sha256Hex;

use super::{
    RunMeta, RunRecord, Store, StoreError, is_valid_sha256_hex, parse_verdict, verdict_token,
};

/// RFC 3339 with exactly millisecond precision — the same shape the
/// trace wire format pins (`event::ts_ms`).
fn fmt_ts(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>, StoreError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| StoreError::BadVerdict(format!("bad timestamp {s:?}")))
}

pub struct SqliteStore {
    pool: Pool<Sqlite>,
    db_path: PathBuf,
    read_only: bool,
}

impl SqliteStore {
    /// Open (creating and migrating if needed) the store at `db_path`.
    /// This is the writable handle — the runtime's.
    pub async fn open(db_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let db_path = db_path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self {
            pool,
            db_path,
            read_only: false,
        })
    }

    /// Open an existing store read-only — the dashboard's handle.
    /// SQLite `mode=ro`: any write through this handle fails at the
    /// connection level. Never creates or migrates.
    pub async fn open_read_only(db_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let db_path = db_path.as_ref().to_path_buf();
        if !db_path.exists() {
            return Err(StoreError::NotFound(db_path));
        }
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .read_only(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        Ok(Self {
            pool,
            db_path,
            read_only: true,
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Escape hatch for tests and #190's scoped queries.
    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }
}

fn row_to_run_record(row: &sqlx::sqlite::SqliteRow) -> Result<RunRecord, StoreError> {
    let inputs_json: String = row.get("inputs");
    let verdict: Option<String> = row.get("verdict");
    let finished_at: Option<String> = row.get("finished_at");
    let duration_ms: Option<i64> = row.get("duration_ms");
    Ok(RunRecord {
        run_id: row.get("run_id"),
        verification: row.get("verification"),
        schema_version: row.get("schema_version"),
        inputs: serde_json::from_str(&inputs_json)?,
        started_at: parse_ts(row.get("started_at"))?,
        verdict: verdict.as_deref().map(parse_verdict).transpose()?,
        finished_at: finished_at.as_deref().map(parse_ts).transpose()?,
        duration_ms: duration_ms.map(|d| d as u64),
    })
}

const RUN_SELECT: &str = "SELECT r.run_id, r.verification, r.schema_version, r.inputs, \
     r.started_at, v.verdict, v.finished_at, v.duration_ms \
     FROM runs r LEFT JOIN run_verdicts v ON v.run_id = r.run_id";

#[async_trait]
impl Store for SqliteStore {
    async fn begin_run(&self, meta: &RunMeta) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO runs (run_id, verification, schema_version, inputs, started_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&meta.run_id)
        .bind(&meta.verification)
        .bind(&meta.schema_version)
        .bind(serde_json::to_string(&meta.inputs)?)
        .bind(fmt_ts(&meta.started_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn append_event(&self, run_id: &str, event: &Event) -> Result<(), StoreError> {
        let line = serde_json::to_string(event)?;
        let mut tx = self.pool.begin().await?;

        sqlx::query("INSERT INTO events (run_id, seq, ts, kind, payload) VALUES (?, ?, ?, ?, ?)")
            .bind(run_id)
            .bind(event.seq as i64)
            .bind(fmt_ts(&event.ts))
            .bind(event.payload.kind())
            .bind(&line)
            .execute(&mut *tx)
            .await?;

        // Fold the derived projection row, if this event carries one.
        // Same transaction: the projections can never drift from the
        // event stream.
        match &event.payload {
            EventPayload::AssertionEvaluated {
                check_id,
                assertion_index,
                state,
                detail,
            } => {
                sqlx::query(
                    "INSERT INTO assertions \
                     (run_id, seq, check_id, assertion_index, state, detail) \
                     VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(run_id)
                .bind(event.seq as i64)
                .bind(check_id)
                .bind(*assertion_index as i64)
                .bind(verdict_token(state)?)
                .bind(detail)
                .execute(&mut *tx)
                .await?;
            }
            EventPayload::CheckFinished { check_id, verdict } => {
                // Resolve the owning criterion from the first
                // step_started that named this check.
                let criterion: Option<String> = sqlx::query_scalar(
                    "SELECT json_extract(payload, '$.criterion_id') FROM events \
                     WHERE run_id = ? AND kind = 'step_started' \
                     AND json_extract(payload, '$.check_id') = ? \
                     ORDER BY seq LIMIT 1",
                )
                .bind(run_id)
                .bind(check_id)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();
                sqlx::query(
                    "INSERT INTO checks (run_id, check_id, criterion_id, verdict) \
                     VALUES (?, ?, ?, ?)",
                )
                .bind(run_id)
                .bind(check_id)
                .bind(criterion)
                .bind(verdict_token(verdict)?)
                .execute(&mut *tx)
                .await?;
            }
            EventPayload::CriterionFinished {
                criterion_id,
                verdict,
            } => {
                sqlx::query(
                    "INSERT INTO criteria (run_id, criterion_id, verdict) VALUES (?, ?, ?)",
                )
                .bind(run_id)
                .bind(criterion_id)
                .bind(verdict_token(verdict)?)
                .execute(&mut *tx)
                .await?;
            }
            EventPayload::RunFinished { verdict } => {
                let started_at: Option<String> =
                    sqlx::query_scalar("SELECT started_at FROM runs WHERE run_id = ?")
                        .bind(run_id)
                        .fetch_optional(&mut *tx)
                        .await?;
                let started_at =
                    started_at.ok_or_else(|| StoreError::UnknownRun(run_id.to_string()))?;
                let duration_ms = (event.ts - parse_ts(&started_at)?)
                    .num_milliseconds()
                    .max(0);
                sqlx::query(
                    "INSERT INTO run_verdicts (run_id, verdict, finished_at, duration_ms) \
                     VALUES (?, ?, ?, ?)",
                )
                .bind(run_id)
                .bind(verdict_token(verdict)?)
                .bind(fmt_ts(&event.ts))
                .bind(duration_ms)
                .execute(&mut *tx)
                .await?;
            }
            _ => {}
        }

        tx.commit().await?;
        Ok(())
    }

    async fn put_blob(&self, bytes: &[u8]) -> Result<Sha256Hex, StoreError> {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let sha = hex::encode(hasher.finalize());
        // Content-addressed: same bytes → same row. OR IGNORE makes
        // re-puts idempotent without ever updating.
        sqlx::query("INSERT OR IGNORE INTO artifacts (sha256, size, bytes) VALUES (?, ?, ?)")
            .bind(&sha)
            .bind(bytes.len() as i64)
            .bind(bytes)
            .execute(&self.pool)
            .await?;
        Ok(Sha256Hex(sha))
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecord>, StoreError> {
        let row = sqlx::query(&format!("{RUN_SELECT} WHERE r.run_id = ?"))
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_run_record).transpose()
    }

    async fn list_runs(&self) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query(&format!(
            "{RUN_SELECT} ORDER BY r.started_at DESC, r.run_id DESC"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_run_record).collect()
    }

    async fn run_events(&self, run_id: &str) -> Result<Vec<Event>, StoreError> {
        self.events_after(run_id, -1).await
    }

    async fn events_after(&self, run_id: &str, after: i64) -> Result<Vec<Event>, StoreError> {
        let lines: Vec<String> = sqlx::query_scalar(
            "SELECT payload FROM events WHERE run_id = ? AND seq > ? ORDER BY seq",
        )
        .bind(run_id)
        .bind(after)
        .fetch_all(&self.pool)
        .await?;
        lines
            .iter()
            .map(|l| serde_json::from_str::<Event>(l).map_err(StoreError::from))
            .collect()
    }

    async fn get_blob(&self, sha256: &str) -> Result<Option<Vec<u8>>, StoreError> {
        if !is_valid_sha256_hex(sha256) {
            return Err(StoreError::BadBlobDigest(sha256.to_string()));
        }
        let bytes: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT bytes FROM artifacts WHERE sha256 = ?")
                .bind(sha256)
                .fetch_optional(&self.pool)
                .await?;
        Ok(bytes)
    }
}
