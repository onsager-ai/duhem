//! Shared fixtures: realistic runs written through the production
//! `EvidenceWriter` into a real `SqliteStore`, so the dashboard tests
//! exercise the same write path `duhem run` produces.
//!
//! Each integration-test binary compiles this module independently
//! and uses a different subset of the fixtures, so the unused-item
//! lint is suppressed at module level.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use duhem_evidence::{
    EventPayload, EvidenceWriter, ObservationValue, SqliteStore, StepOutcome, VerdictState,
    run_started,
};
use duhem_judge::InconclusiveCause;

pub const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

pub fn png_bytes() -> Vec<u8> {
    let mut bytes = PNG_MAGIC.to_vec();
    bytes.extend(std::iter::repeat_n(0u8, 64));
    bytes
}

/// A fresh store in a tempdir, plus a read-only handle onto the same
/// DB (the dashboard's view).
pub async fn open_stores() -> (tempfile::TempDir, Arc<SqliteStore>, Arc<SqliteStore>) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db = tmp.path().join("duhem.db");
    let rw = Arc::new(SqliteStore::open(&db).await.expect("open store"));
    let ro = Arc::new(
        SqliteStore::open_read_only(&db)
            .await
            .expect("open read-only store"),
    );
    (tmp, rw, ro)
}

fn inputs() -> BTreeMap<String, serde_json::Value> {
    let mut m = BTreeMap::new();
    m.insert("workspace_name".into(), serde_json::json!("ws-fixture"));
    m
}

/// A finished, passing run with env + setup + one criterion (AC-1)
/// holding one check (AC-1.1) of two steps; the second step records a
/// PNG blob artifact. Returns the blob's sha-256.
pub async fn write_passing_run(
    store: Arc<SqliteStore>,
    run_id: &str,
    definition_path: &str,
) -> String {
    let mut w = EvidenceWriter::begin(store, run_id, definition_path, inputs())
        .await
        .expect("writer");
    w.append(run_started(definition_path, inputs()))
        .await
        .unwrap();
    w.append(EventPayload::EnvUpStarted {
        command: "./up.sh".into(),
    })
    .await
    .unwrap();
    w.append(EventPayload::EnvUpFinished {
        exit_code: 0,
        duration_ms: 120,
        stdout_blob_sha256: None,
        stderr_blob_sha256: None,
    })
    .await
    .unwrap();
    w.append(EventPayload::EnvReady {
        probe_kind: "http".into(),
        ok: true,
        elapsed_ms: 30,
    })
    .await
    .unwrap();
    w.append(EventPayload::SetupStarted { step_count: 1 })
        .await
        .unwrap();
    w.append(EventPayload::SetupStepStarted {
        step_index: 0,
        uses: "ui/navigate".into(),
        layer: None,
        with: BTreeMap::new(),
    })
    .await
    .unwrap();
    w.append(EventPayload::SetupStepObservation {
        step_index: 0,
        output_name: "landed_at".into(),
        value: ObservationValue::Inline {
            value: serde_json::json!("http://sut/"),
        },
    })
    .await
    .unwrap();
    w.append(EventPayload::SetupStepFinished {
        step_index: 0,
        outcome: StepOutcome::Ok,
    })
    .await
    .unwrap();
    w.append(EventPayload::SetupFinished { aborted: false })
        .await
        .unwrap();

    w.append(EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: 0,
        uses: "ui/navigate".into(),
        layer: Some("ui".into()),
        with: BTreeMap::new(),
    })
    .await
    .unwrap();
    w.append(EventPayload::StepObservation {
        step_index: 0,
        output_name: "landed_at".into(),
        value: ObservationValue::Inline {
            value: serde_json::json!("http://sut/projects"),
        },
    })
    .await
    .unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 0,
        outcome: StepOutcome::Ok,
    })
    .await
    .unwrap();

    let sha = w.write_blob(&png_bytes()).await.unwrap();
    w.append(EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: 1,
        uses: "ui/assert-element".into(),
        layer: Some("ui".into()),
        with: BTreeMap::new(),
    })
    .await
    .unwrap();
    w.append(EventPayload::StepObservation {
        step_index: 1,
        output_name: "screenshot".into(),
        value: ObservationValue::Blob {
            blob_sha256: sha.as_str().to_string(),
        },
    })
    .await
    .unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 1,
        outcome: StepOutcome::Ok,
    })
    .await
    .unwrap();

    w.append(EventPayload::AssertionEvaluated {
        check_id: "AC-1.1".into(),
        assertion_index: 0,
        state: VerdictState::Pass,
        detail: None,
        expr: None,
        step_index: None,
    })
    .await
    .unwrap();
    w.append(EventPayload::CheckFinished {
        check_id: "AC-1.1".into(),
        verdict: VerdictState::Pass,
    })
    .await
    .unwrap();
    w.append(EventPayload::CriterionFinished {
        criterion_id: "AC-1".into(),
        verdict: VerdictState::Pass,
    })
    .await
    .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Pass,
    })
    .await
    .unwrap();
    w.finish().await.unwrap();
    sha.as_str().to_string()
}

/// A finished failing run: one criterion, one check, one failing
/// assertion.
pub async fn write_failing_run(store: Arc<SqliteStore>, run_id: &str, definition_path: &str) {
    let mut w = EvidenceWriter::begin(store, run_id, definition_path, BTreeMap::new())
        .await
        .expect("writer");
    w.append(run_started(definition_path, BTreeMap::new()))
        .await
        .unwrap();
    w.append(EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: 0,
        uses: "api/call".into(),
        layer: Some("api".into()),
        with: BTreeMap::new(),
    })
    .await
    .unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 0,
        outcome: StepOutcome::Ok,
    })
    .await
    .unwrap();
    w.append(EventPayload::AssertionEvaluated {
        check_id: "AC-1.1".into(),
        assertion_index: 0,
        state: VerdictState::Fail,
        detail: Some("status 500 != 200".into()),
        expr: None,
        step_index: None,
    })
    .await
    .unwrap();
    w.append(EventPayload::CheckFinished {
        check_id: "AC-1.1".into(),
        verdict: VerdictState::Fail,
    })
    .await
    .unwrap();
    w.append(EventPayload::CriterionFinished {
        criterion_id: "AC-1".into(),
        verdict: VerdictState::Fail,
    })
    .await
    .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Fail,
    })
    .await
    .unwrap();
    w.finish().await.unwrap();
}

/// A setup-aborted run, mirroring the engine's abort path (#20): no
/// checks ran, `setup_finished { aborted: true }`, run verdict
/// `inconclusive:environment_error`.
pub async fn write_aborted_run(store: Arc<SqliteStore>, run_id: &str, definition_path: &str) {
    let mut w = EvidenceWriter::begin(store, run_id, definition_path, BTreeMap::new())
        .await
        .expect("writer");
    w.append(run_started(definition_path, BTreeMap::new()))
        .await
        .unwrap();
    w.append(EventPayload::SetupStarted { step_count: 1 })
        .await
        .unwrap();
    w.append(EventPayload::SetupStepStarted {
        step_index: 0,
        uses: "api/call".into(),
        layer: None,
        with: BTreeMap::new(),
    })
    .await
    .unwrap();
    w.append(EventPayload::SetupStepFinished {
        step_index: 0,
        outcome: StepOutcome::Error,
    })
    .await
    .unwrap();
    w.append(EventPayload::SetupFinished { aborted: true })
        .await
        .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
    })
    .await
    .unwrap();
    w.finish().await.unwrap();
}

/// An in-progress run (#84): a step has started, no `run_finished` —
/// the store-era "live" shape (no verdict row).
pub async fn write_in_progress_run(store: Arc<SqliteStore>, run_id: &str, definition_path: &str) {
    let mut w = EvidenceWriter::begin(store, run_id, definition_path, BTreeMap::new())
        .await
        .expect("writer");
    w.append(run_started(definition_path, BTreeMap::new()))
        .await
        .unwrap();
    w.append(EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: 0,
        uses: "ui/navigate".into(),
        layer: None,
        with: BTreeMap::new(),
    })
    .await
    .unwrap();
    // No run_finished — the run stays live/unfinished in the store.
}
