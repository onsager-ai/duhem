//! The `Store` trait — the single-source-of-truth seam (#189).
//!
//! Runs live in a database, not in per-run JSONL files. The runtime
//! (which owns the judge's verdicts) is the **sole writer**; the
//! dashboard opens a **read-only handle** and can never mutate. The
//! trait is the open-core boundary from #188: `SqliteStore` (local,
//! this crate) and a future hosted `PostgresStore` implement the same
//! surface.
//!
//! Write surface: [`Store::begin_run`] → [`Store::append_event`]* →
//! (the `run_finished` event seals the run — its fold inserts the
//! verdict row, after which further appends are rejected by the DB).
//! Read surface: [`Store::get_run`] / [`Store::list_runs`] /
//! [`Store::run_events`] / [`Store::events_after`] /
//! [`Store::get_blob`].

mod location;
mod sqlite;

pub use location::{project_db_path, project_slug, state_root};
pub use sqlite::SqliteStore;

use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::event::{Event, VerdictState};
use crate::writer::Sha256Hex;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("serialize error: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no duhem store at {0} (no runs recorded here yet)")]
    NotFound(PathBuf),
    #[error("unknown run: {0}")]
    UnknownRun(String),
    #[error("blob digest {0:?} is not a 64-char lowercase hex sha-256")]
    BadBlobDigest(String),
    #[error("cannot resolve a home/state directory for the duhem store")]
    NoStateDir,
    #[error("bad verdict token {0:?} in store")]
    BadVerdict(String),
}

/// Header recorded at `begin_run` — the store-level successor of the
/// on-disk `manifest.json`. The `run_started` event redundantly
/// carries the same facts so an exported event stream stays
/// self-describing.
#[derive(Debug, Clone)]
pub struct RunMeta {
    pub run_id: String,
    /// Definition path — which verification this run executed.
    pub verification: String,
    /// Trace wire version (`duhem_evidence::SCHEMA_VERSION`).
    pub schema_version: String,
    pub inputs: BTreeMap<String, serde_json::Value>,
    pub started_at: DateTime<Utc>,
}

/// One run as the store knows it: the `runs` row joined with its
/// verdict row, if judgment has landed. `verdict: None` means the run
/// is in flight or crashed before `run_finished` — the same semantics
/// a trace without a final line had.
#[derive(Debug, Clone)]
pub struct RunRecord {
    pub run_id: String,
    pub verification: String,
    pub schema_version: String,
    pub inputs: BTreeMap<String, serde_json::Value>,
    pub started_at: DateTime<Utc>,
    pub verdict: Option<VerdictState>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
}

/// The storage seam. Object-safe (`Arc<dyn Store>`) so the runtime,
/// CLI, and dashboard stay backend-agnostic.
#[async_trait]
pub trait Store: Send + Sync {
    /// Record the run header. Must be called once before any
    /// `append_event` for that run.
    async fn begin_run(&self, meta: &RunMeta) -> Result<(), StoreError>;

    /// Append one event (seq/ts already stamped by the writer) and
    /// fold its derived projection rows (assertions, checks, criteria,
    /// run verdict) in the same transaction. Appending the
    /// `run_finished` event seals the run.
    async fn append_event(&self, run_id: &str, event: &Event) -> Result<(), StoreError>;

    /// Store a content-addressed blob. Idempotent: identical content
    /// returns the same address without rewriting.
    async fn put_blob(&self, bytes: &[u8]) -> Result<Sha256Hex, StoreError>;

    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecord>, StoreError>;

    /// All runs, most recent first.
    async fn list_runs(&self) -> Result<Vec<RunRecord>, StoreError>;

    /// The full event stream for a run, in seq order — the replay
    /// input and the export source.
    async fn run_events(&self, run_id: &str) -> Result<Vec<Event>, StoreError>;

    /// Events with `seq > after`, in seq order. `after = -1` returns
    /// the full stream. This is the live-tail primitive for SSE.
    async fn events_after(&self, run_id: &str, after: i64) -> Result<Vec<Event>, StoreError>;

    async fn get_blob(&self, sha256: &str) -> Result<Option<Vec<u8>>, StoreError>;
}

/// Serialize a verdict to its bare wire token (`pass` / `fail` /
/// `inconclusive:<cause>`) for storage in a TEXT column — queryable
/// without JSON quoting.
pub(crate) fn verdict_token(v: &VerdictState) -> Result<String, StoreError> {
    match serde_json::to_value(v)? {
        serde_json::Value::String(s) => Ok(s),
        other => Ok(other.to_string()),
    }
}

pub(crate) fn parse_verdict(token: &str) -> Result<VerdictState, StoreError> {
    serde_json::from_value(serde_json::Value::String(token.to_string()))
        .map_err(|_| StoreError::BadVerdict(token.to_string()))
}

pub(crate) fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
