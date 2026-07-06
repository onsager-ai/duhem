//! The run-bundle — Duhem's portability + hub-ingest wire format
//! (#189 export, #194 wire contract).
//!
//! One immutable run, self-contained: the run header (manifest
//! successor + #190 scope/provenance), the full wire-format event
//! stream (#10), and every content-addressed artifact the stream
//! references. **One format, two destinations**: `duhem export`
//! writes it as a directory a human can browse; `duhem ship` POSTs
//! it as a single canonical JSON envelope the hub ingests. Both are
//! renderings of [`RunBundle`], so they cannot drift.
//!
//! The format is versioned by [`BUNDLE_VERSION`], decoupled from the
//! VD `SCHEMA_VERSION` and pinned by the cross-repo contract test in
//! `tests/bundle_contract.rs` — the closed-source hub builds against
//! this shape (#188 open-core seam). The bundle's **content hash**
//! (sha-256 of the canonical envelope bytes) is its idempotency key:
//! re-shipping the same run is a server-side no-op.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::event::{Event, EventPayload, ObservationValue, VerdictState};
use crate::store::{RunRecord, Store, StoreError};

/// Version of the bundle wire shape. Breaking shape changes bump it;
/// the hub refuses versions it doesn't understand rather than
/// misparse them.
pub const BUNDLE_VERSION: u32 = 1;

/// The envelope. Serialized with `serde_json::to_vec` (compact, keys
/// in struct order) for the canonical wire bytes; pretty-printed
/// inside the export directory for humans.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunBundle {
    pub bundle_version: u32,
    pub run: BundleRun,
    /// The full wire-format stream, seq order — replay input.
    pub events: Vec<Event>,
    /// Content-addressed blobs the stream references, base64 bytes.
    /// Sorted by sha so the envelope is deterministic.
    pub artifacts: Vec<BundleArtifact>,
}

/// The run header — the store row, without the store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BundleRun {
    pub run_id: String,
    pub verification: String,
    pub schema_version: String,
    pub inputs: std::collections::BTreeMap<String, serde_json::Value>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<VerdictState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// #190 scope + provenance, flattened as optional fields so an
    /// unattributed run serializes compactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier_repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_sha: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BundleArtifact {
    pub sha256: String,
    /// Base64 (standard alphabet, padded) of the blob bytes.
    pub bytes_base64: String,
}

fn fmt_ts(dt: &chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

impl RunBundle {
    /// Assemble the bundle for one recorded run: header from the run
    /// row, the full event stream, and every blob the stream
    /// references (sorted by sha for determinism).
    pub async fn from_store(store: &dyn Store, run_id: &str) -> Result<Self, StoreError> {
        let record = store
            .get_run(run_id)
            .await?
            .ok_or_else(|| StoreError::UnknownRun(run_id.to_string()))?;
        let events = store.run_events(run_id).await?;
        let mut shas = referenced_blobs(&events);
        shas.sort();
        let mut artifacts = Vec::with_capacity(shas.len());
        for sha in shas {
            let bytes = store
                .get_blob(&sha)
                .await?
                .ok_or_else(|| StoreError::UnknownRun(format!("missing artifact {sha}")))?;
            artifacts.push(BundleArtifact {
                sha256: sha,
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            });
        }
        Ok(Self {
            bundle_version: BUNDLE_VERSION,
            run: bundle_run(&record),
            events,
            artifacts,
        })
    }

    /// The canonical wire bytes (compact JSON). What `duhem ship`
    /// POSTs and what the content hash is computed over.
    pub fn wire_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// The bundle's idempotency key: lowercase-hex sha-256 of the
    /// canonical wire bytes.
    pub fn content_hash(&self) -> Result<String, serde_json::Error> {
        let mut hasher = Sha256::new();
        hasher.update(self.wire_bytes()?);
        Ok(hex::encode(hasher.finalize()))
    }

    /// Write the human-browsable export layout (#189):
    ///
    /// ```text
    /// <out>/bundle.json          # this envelope minus events/artifact bytes
    /// <out>/events.jsonl         # one wire event per line
    /// <out>/artifacts/<sha256>   # raw blob bytes
    /// ```
    pub fn write_dir(&self, out: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(out)?;
        // Header document: the envelope shape with the bulky parts
        // externalized to their files; carries the content hash so a
        // directory round-trips verifiably.
        let header = serde_json::json!({
            "bundle_version": self.bundle_version,
            "content_hash": self.content_hash().map_err(std::io::Error::other)?,
            "run": self.run,
            "event_count": self.events.len(),
            "artifact_count": self.artifacts.len(),
        });
        std::fs::write(
            out.join("bundle.json"),
            serde_json::to_vec_pretty(&header).map_err(std::io::Error::other)?,
        )?;
        let mut jsonl = String::new();
        for e in &self.events {
            jsonl.push_str(&serde_json::to_string(e).map_err(std::io::Error::other)?);
            jsonl.push('\n');
        }
        std::fs::write(out.join("events.jsonl"), jsonl)?;
        if !self.artifacts.is_empty() {
            let dir = out.join("artifacts");
            std::fs::create_dir_all(&dir)?;
            for a in &self.artifacts {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&a.bytes_base64)
                    .map_err(std::io::Error::other)?;
                std::fs::write(dir.join(&a.sha256), bytes)?;
            }
        }
        Ok(())
    }

    /// Read an export directory back into the envelope. Round-trips
    /// with [`RunBundle::write_dir`] — the same-format guarantee the
    /// export/ship pair rests on.
    pub fn from_dir(dir: &Path) -> std::io::Result<Self> {
        let header: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("bundle.json"))?)
                .map_err(std::io::Error::other)?;
        let bundle_version = header
            .get("bundle_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let run: BundleRun = serde_json::from_value(
            header
                .get("run")
                .cloned()
                .ok_or_else(|| std::io::Error::other("bundle.json missing run"))?,
        )
        .map_err(std::io::Error::other)?;
        let jsonl = std::fs::read_to_string(dir.join("events.jsonl"))?;
        let mut events = Vec::new();
        for line in jsonl.lines() {
            if line.is_empty() {
                continue;
            }
            events.push(serde_json::from_str(line).map_err(std::io::Error::other)?);
        }
        let mut artifacts = Vec::new();
        let art_dir = dir.join("artifacts");
        if art_dir.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(&art_dir)?
                .filter_map(|e| e.ok())
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let sha = entry.file_name().to_string_lossy().into_owned();
                let bytes = std::fs::read(entry.path())?;
                artifacts.push(BundleArtifact {
                    sha256: sha,
                    bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                });
            }
        }
        Ok(Self {
            bundle_version,
            run,
            events,
            artifacts,
        })
    }
}

fn bundle_run(record: &RunRecord) -> BundleRun {
    BundleRun {
        run_id: record.run_id.clone(),
        verification: record.verification.clone(),
        schema_version: record.schema_version.clone(),
        inputs: record.inputs.clone(),
        started_at: fmt_ts(&record.started_at),
        verdict: record.verdict,
        finished_at: record.finished_at.as_ref().map(fmt_ts),
        duration_ms: record.duration_ms,
        project_id: record.scope.project_id.clone(),
        verifier_repo: record.scope.verifier_repo.clone(),
        verifier_sha: record.scope.verifier_sha.clone(),
        target_repo: record.scope.target_repo.clone(),
        target_sha: record.scope.target_sha.clone(),
    }
}

/// Every content address the event stream references — observation
/// blobs plus captured env stdio.
pub fn referenced_blobs(events: &[Event]) -> Vec<String> {
    let mut shas: Vec<String> = Vec::new();
    let mut push = |sha: &String| {
        if !shas.contains(sha) {
            shas.push(sha.clone());
        }
    };
    for evt in events {
        match &evt.payload {
            EventPayload::StepObservation {
                value: ObservationValue::Blob { blob_sha256 },
                ..
            }
            | EventPayload::SetupStepObservation {
                value: ObservationValue::Blob { blob_sha256 },
                ..
            } => push(blob_sha256),
            EventPayload::EnvUpFinished {
                stdout_blob_sha256,
                stderr_blob_sha256,
                ..
            }
            | EventPayload::EnvDownFinished {
                stdout_blob_sha256,
                stderr_blob_sha256,
                ..
            } => {
                if let Some(s) = stdout_blob_sha256 {
                    push(s);
                }
                if let Some(s) = stderr_blob_sha256 {
                    push(s);
                }
            }
            _ => {}
        }
    }
    shas
}
