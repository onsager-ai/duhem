//! `duhem export` (#189): write one run out of the store as a
//! self-contained bundle — the portability path now that evidence
//! lives in a DB instead of per-run files.
//!
//! Bundle layout under `--out` (default `duhem-export-<run-id>/`):
//!
//! ```text
//! bundle.json         # bundle_version + run header + verdict
//! events.jsonl        # the wire-format event stream (#10 shape)
//! artifacts/<sha256>  # content-addressed blobs the stream references
//! ```
//!
//! The stream + artifacts are everything replay and rendering need;
//! `bundle.json` carries the store-level header so the bundle is
//! self-describing without the DB row next to it. The bundle format
//! is versioned (`bundle_version`) — the hub ingest contract (#194)
//! formalizes it as a wire contract; this is version 1.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use duhem_evidence::{Event, EventPayload, ObservationValue, RunRecord, SqliteStore, Store};

/// Version of the bundle layout. Bumped on breaking shape changes;
/// #194 pins it with a cross-repo contract test.
pub const BUNDLE_VERSION: u32 = 1;

pub async fn run_export(run_id: &str, db: Option<&Path>, out: Option<&Path>) -> ExitCode {
    match export_run(run_id, db, out).await {
        Ok(out_dir) => {
            println!("exported run {run_id} to {}", out_dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("export: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn export_run(
    run_id: &str,
    db: Option<&Path>,
    out: Option<&Path>,
) -> Result<PathBuf, String> {
    let db_path = match db {
        Some(p) => p.to_path_buf(),
        None => {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            duhem_evidence::project_db_path(&cwd).map_err(|e| e.to_string())?
        }
    };
    // Read-only: an export must never mutate the store.
    let store = SqliteStore::open_read_only(&db_path)
        .await
        .map_err(|e| e.to_string())?;

    let record = store
        .get_run(run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("unknown run: {run_id} (store: {})", db_path.display()))?;
    let events = store.run_events(run_id).await.map_err(|e| e.to_string())?;

    let out_dir = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(format!("duhem-export-{run_id}")));
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    // bundle.json — the run header (manifest successor) + verdict.
    let bundle = bundle_header(&record);
    std::fs::write(
        out_dir.join("bundle.json"),
        serde_json::to_vec_pretty(&bundle).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    // events.jsonl — the wire-format stream, one event per line.
    let mut jsonl = String::new();
    for evt in &events {
        jsonl.push_str(&serde_json::to_string(evt).map_err(|e| e.to_string())?);
        jsonl.push('\n');
    }
    std::fs::write(out_dir.join("events.jsonl"), jsonl).map_err(|e| e.to_string())?;

    // artifacts/<sha256> — every blob the stream references.
    let shas = referenced_blobs(&events);
    if !shas.is_empty() {
        let artifacts_dir = out_dir.join("artifacts");
        std::fs::create_dir_all(&artifacts_dir).map_err(|e| e.to_string())?;
        for sha in shas {
            let bytes = store
                .get_blob(&sha)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("stream references missing artifact {sha}"))?;
            std::fs::write(artifacts_dir.join(&sha), bytes).map_err(|e| e.to_string())?;
        }
    }

    Ok(out_dir)
}

fn bundle_header(record: &RunRecord) -> serde_json::Value {
    serde_json::json!({
        "bundle_version": BUNDLE_VERSION,
        "run": {
            "run_id": record.run_id,
            "verification": record.verification,
            "schema_version": record.schema_version,
            "inputs": record.inputs,
            "started_at": record.started_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            "verdict": record.verdict.map(|v| v.to_string()),
            "finished_at": record
                .finished_at
                .map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()),
            "duration_ms": record.duration_ms,
        },
    })
}

/// Every content address the event stream references — observation
/// blobs plus captured env stdio.
fn referenced_blobs(events: &[Event]) -> Vec<String> {
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
