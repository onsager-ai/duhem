//! Append-only writer for `trace.jsonl` plus the content-addressed
//! blob store under `blobs/`.
//!
//! Contract (from issue #10):
//!
//! - `trace.jsonl` is opened with `O_APPEND`. One writer per run; the
//!   runtime owns the handle.
//! - Every line ends with `\n`. Crash mid-write leaves zero half-lines
//!   at EOF (we build the whole line in memory, then issue a single
//!   `write` — short of a partial-write at the filesystem level, the
//!   reader sees only complete records).
//! - `fsync` at every `*_finished` event and at `run_finished`. Step
//!   observations buffer — losing the last few on crash is acceptable,
//!   losing a verdict is not.
//! - Blob writes are write-then-rename
//!   (`blobs/.tmp.<sha>` → `blobs/<sha>`) so readers polling the
//!   directory never see a partial file.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SubsecRound, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::event::{
    BLOB_INLINE_THRESHOLD_BYTES, Event, EventPayload, ObservationValue, SCHEMA_VERSION, ts_ms,
};

/// Best-effort directory fsync. After a `rename` of a temp file into
/// place, the directory entry itself isn't durable until the
/// containing directory is synced. POSIX-only — no-op on platforms
/// where opening a directory as a file isn't supported.
fn fsync_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Truncate to millisecond precision. The on-disk format pins `ts` at
/// ms; in-memory `Utc::now()` carries ns. Truncate at the stamping
/// boundary so the value matches the wire form exactly.
fn now_ms() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(3)
}

#[derive(Debug, Error)]
pub enum WriterError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialize error: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("run directory already contains a trace: {0}")]
    AlreadyExists(PathBuf),
}

/// SHA-256 digest of a blob, as lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Hex(pub String);

impl Sha256Hex {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The run-level summary written to `manifest.json` once at creation.
/// The manifest is allowed to be lost without breaking replay — every
/// `run_started` event redundantly carries `schema_version`.
#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    pub run_id: String,
    #[serde(with = "ts_ms")]
    pub started_at: DateTime<Utc>,
    pub definition_path: String,
    pub schema_version: String,
}

/// Append-only writer for a single run.
pub struct EvidenceWriter {
    run_dir: PathBuf,
    trace: File,
    next_seq: u64,
}

impl EvidenceWriter {
    /// Create a new run directory and open `trace.jsonl` for append.
    ///
    /// `definition_path` is recorded in the manifest only; the
    /// run-level `run_started` event (with the same path) is emitted
    /// by the caller as its first `append` call. This split lets the
    /// runtime decide what to put in `inputs` at the event level while
    /// the writer stays agnostic.
    pub fn new(run_dir: impl AsRef<Path>, definition_path: &str) -> Result<Self, WriterError> {
        let run_dir = run_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&run_dir)?;
        std::fs::create_dir_all(run_dir.join("blobs"))?;

        let trace_path = run_dir.join("trace.jsonl");
        if trace_path.exists() {
            return Err(WriterError::AlreadyExists(trace_path));
        }
        let trace = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&trace_path)?;

        let run_id = run_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let manifest = Manifest {
            run_id,
            started_at: now_ms(),
            definition_path: definition_path.to_string(),
            schema_version: SCHEMA_VERSION.to_string(),
        };
        let manifest_json = serde_json::to_vec_pretty(&manifest)?;
        let manifest_tmp = run_dir.join(".manifest.json.tmp");
        let manifest_final = run_dir.join("manifest.json");
        {
            let mut f = File::create(&manifest_tmp)?;
            f.write_all(&manifest_json)?;
            f.sync_all()?;
        }
        std::fs::rename(&manifest_tmp, &manifest_final)?;
        // The directory entry for trace.jsonl, blobs/, and the
        // renamed manifest.json all need a directory fsync to be
        // durable across a crash.
        fsync_dir(&run_dir)?;

        Ok(Self {
            run_dir,
            trace,
            next_seq: 0,
        })
    }

    /// The run directory this writer is operating on.
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// Append one event. The caller supplies the `payload`; `seq` and
    /// `ts` are stamped here so monotonicity is the writer's
    /// responsibility, not the caller's.
    pub fn append(&mut self, payload: EventPayload) -> Result<u64, WriterError> {
        let needs_fsync = payload.is_finished();

        let evt = Event {
            seq: self.next_seq,
            ts: now_ms(),
            payload,
        };
        let mut line = serde_json::to_vec(&evt)?;
        line.push(b'\n');
        self.trace.write_all(&line)?;
        if needs_fsync {
            self.trace.sync_data()?;
        }

        let seq = self.next_seq;
        self.next_seq += 1;
        Ok(seq)
    }

    /// Convenience: emit a `step_observation`, choosing inline vs
    /// blob automatically based on the serialized byte length of
    /// `value` against [`BLOB_INLINE_THRESHOLD_BYTES`].
    pub fn append_observation(
        &mut self,
        step_index: u32,
        output_name: impl Into<String>,
        value: serde_json::Value,
    ) -> Result<u64, WriterError> {
        let inline_bytes = serde_json::to_vec(&value)?;
        let obs = if inline_bytes.len() > BLOB_INLINE_THRESHOLD_BYTES {
            let sha = self.write_blob(&inline_bytes)?;
            ObservationValue::Blob { blob_sha256: sha.0 }
        } else {
            ObservationValue::Inline { value }
        };
        self.append(EventPayload::StepObservation {
            step_index,
            output_name: output_name.into(),
            value: obs,
        })
    }

    /// Write a blob to `blobs/<sha256>` and return its content
    /// address. Write-then-rename so a reader polling the directory
    /// never sees a partial file. Idempotent: if the blob already
    /// exists, the temp file is discarded.
    pub fn write_blob(&mut self, bytes: &[u8]) -> Result<Sha256Hex, WriterError> {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let sha = hex::encode(hasher.finalize());

        let final_path = self.run_dir.join("blobs").join(&sha);
        if final_path.exists() {
            return Ok(Sha256Hex(sha));
        }
        let tmp_path = self.run_dir.join("blobs").join(format!(".tmp.{sha}"));
        {
            let mut f = File::create(&tmp_path)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp_path, &final_path)?;
        fsync_dir(&self.run_dir.join("blobs"))?;
        Ok(Sha256Hex(sha))
    }

    /// Flush and close. Safe to drop without calling `finish` — the
    /// crash-recovery test relies on that — but `finish` is the only
    /// way to guarantee the final state is fsynced if the last event
    /// wasn't a `*_finished` kind.
    pub fn finish(self) -> Result<(), WriterError> {
        self.trace.sync_all()?;
        Ok(())
    }
}

/// Helper for building a `run_started` payload without hand-rolling
/// `BTreeMap` everywhere.
pub fn run_started(
    verification_path: impl Into<String>,
    inputs: BTreeMap<String, serde_json::Value>,
) -> EventPayload {
    EventPayload::RunStarted {
        verification_path: verification_path.into(),
        inputs,
        schema_version: SCHEMA_VERSION.to_string(),
    }
}
