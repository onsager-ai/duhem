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
    CriterionHistoryEntry, ProjectSummary, RunMeta, RunRecord, RunScope, Store, StoreError,
    TargetStatus, is_valid_sha256_hex, parse_verdict, verdict_token, verification_key,
    verification_name,
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
        scope: RunScope {
            project_id: row.get("project_id"),
            verifier_repo: row.get("verifier_repo"),
            verifier_sha: row.get("verifier_sha"),
            target_repo: row.get("target_repo"),
            target_sha: row.get("target_sha"),
        },
    })
}

const RUN_SELECT: &str = "SELECT r.run_id, r.verification, r.schema_version, r.inputs, \
     r.started_at, r.project_id, r.verifier_repo, r.verifier_sha, r.target_repo, \
     r.target_sha, v.verdict, v.finished_at, v.duration_ms \
     FROM runs r LEFT JOIN run_verdicts v ON v.run_id = r.run_id";

#[async_trait]
impl Store for SqliteStore {
    async fn begin_run(&self, meta: &RunMeta) -> Result<(), StoreError> {
        let name = verification_name(&meta.verification);
        let verification_id = verification_key(meta.scope.project_id.as_deref(), &name);
        let mut tx = self.pool.begin().await?;

        // Fold the dimension rows first (idempotent identity upserts),
        // then the run row referencing them — one transaction, so the
        // writer's referential integrity can't be observed half-done.
        if let Some(project) = &meta.scope.project_id {
            sqlx::query(
                "INSERT OR IGNORE INTO projects (project_id, workspace_id) VALUES (?, 'local')",
            )
            .bind(project)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT OR IGNORE INTO verifications \
             (verification_id, project_id, name, definition_path) VALUES (?, ?, ?, ?)",
        )
        .bind(&verification_id)
        .bind(&meta.scope.project_id)
        .bind(&name)
        .bind(&meta.verification)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO runs (run_id, verification, schema_version, inputs, started_at, \
             workspace_id, project_id, verification_id, verifier_repo, verifier_sha, \
             target_repo, target_sha) \
             VALUES (?, ?, ?, ?, ?, 'local', ?, ?, ?, ?, ?, ?)",
        )
        .bind(&meta.run_id)
        .bind(&meta.verification)
        .bind(&meta.schema_version)
        .bind(serde_json::to_string(&meta.inputs)?)
        .bind(fmt_ts(&meta.started_at))
        .bind(&meta.scope.project_id)
        .bind(&verification_id)
        .bind(&meta.scope.verifier_repo)
        .bind(&meta.scope.verifier_sha)
        .bind(&meta.scope.target_repo)
        .bind(&meta.scope.target_sha)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
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

    async fn portfolio(&self) -> Result<Vec<ProjectSummary>, StoreError> {
        // Latest run per project via a correlated pick on
        // (started_at, run_id) — ULIDs tie-break identically-stamped
        // rows chronologically.
        let rows = sqlx::query(
            "SELECT r.project_id, COUNT(*) AS run_count, \
             COUNT(DISTINCT r.verification_id) AS verification_count, \
             l.run_id AS latest_run_id, l.started_at AS latest_started_at, \
             lv.verdict AS latest_verdict \
             FROM runs r \
             LEFT JOIN runs l ON l.run_id = ( \
                 SELECT r2.run_id FROM runs r2 \
                 WHERE r2.project_id IS r.project_id \
                 ORDER BY r2.started_at DESC, r2.run_id DESC LIMIT 1) \
             LEFT JOIN run_verdicts lv ON lv.run_id = l.run_id \
             GROUP BY r.project_id \
             ORDER BY (r.project_id IS NULL), MAX(r.started_at) DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let latest_started_at: Option<String> = row.get("latest_started_at");
                let latest_verdict: Option<String> = row.get("latest_verdict");
                Ok(ProjectSummary {
                    project_id: row.get("project_id"),
                    run_count: row.get::<i64, _>("run_count") as u64,
                    verification_count: row.get::<i64, _>("verification_count") as u64,
                    latest_run_id: row.get("latest_run_id"),
                    latest_started_at: latest_started_at.as_deref().map(parse_ts).transpose()?,
                    latest_verdict: latest_verdict.as_deref().map(parse_verdict).transpose()?,
                })
            })
            .collect()
    }

    async fn verification_history(&self, name: &str) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query(&format!(
            "{RUN_SELECT} JOIN verifications vf ON vf.verification_id = r.verification_id \
             WHERE vf.name = ? ORDER BY r.started_at DESC, r.run_id DESC"
        ))
        .bind(name)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_run_record).collect()
    }

    async fn criterion_history(
        &self,
        name: &str,
    ) -> Result<Vec<CriterionHistoryEntry>, StoreError> {
        let rows = sqlx::query(
            "SELECT c.run_id, r.started_at, c.criterion_id, c.verdict \
             FROM criteria c \
             JOIN runs r ON r.run_id = c.run_id \
             JOIN verifications vf ON vf.verification_id = r.verification_id \
             WHERE vf.name = ? \
             ORDER BY r.started_at DESC, r.run_id DESC, c.criterion_id",
        )
        .bind(name)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(CriterionHistoryEntry {
                    run_id: row.get("run_id"),
                    started_at: parse_ts(row.get("started_at"))?,
                    criterion_id: row.get("criterion_id"),
                    verdict: parse_verdict(row.get("verdict"))?,
                })
            })
            .collect()
    }

    async fn target_status(
        &self,
        target_repo: &str,
        target_sha: &str,
    ) -> Result<Option<TargetStatus>, StoreError> {
        let row = sqlx::query(
            "SELECT r.run_id, v.verdict FROM runs r \
             LEFT JOIN run_verdicts v ON v.run_id = r.run_id \
             WHERE r.target_repo = ? AND r.target_sha = ? \
             ORDER BY r.started_at DESC, r.run_id DESC LIMIT 1",
        )
        .bind(target_repo)
        .bind(target_sha)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let verdict: Option<String> = row.get("verdict");
        let latest_verdict = verdict.as_deref().map(parse_verdict).transpose()?;
        Ok(Some(TargetStatus {
            target_repo: target_repo.to_string(),
            target_sha: target_sha.to_string(),
            latest_run_id: row.get("run_id"),
            latest_verdict,
            // Only a recorded pass unblocks; an unfinished run is not
            // attestation.
            blocked: latest_verdict != Some(crate::event::VerdictState::Pass),
        }))
    }
}
