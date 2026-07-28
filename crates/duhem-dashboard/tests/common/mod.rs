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
        verdict: Some(VerdictState::Pass),
    })
    .await
    .unwrap();
    w.finish().await.unwrap();
    sha.as_str().to_string()
}

/// A passing browser check carrying one retained storyboard frame and a
/// versioned session-evidence document (#337).
pub async fn write_replay_run(store: Arc<SqliteStore>, run_id: &str) -> (String, String) {
    use duhem_evidence::{
        SESSION_EVIDENCE_OBSERVATION, STEP_SCREENSHOT_OBSERVATION, SessionEvidence,
        SessionNetworkEntry, SessionPerformanceObservation, SessionStep, SessionVideo,
    };

    let mut w = EvidenceWriter::begin(store, run_id, "replay.yml", BTreeMap::new())
        .await
        .unwrap();
    w.append(run_started("replay.yml", BTreeMap::new()))
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
    w.append(EventPayload::StepFinished {
        step_index: 0,
        outcome: StepOutcome::Ok,
    })
    .await
    .unwrap();
    let shot = w.write_blob(&png_bytes()).await.unwrap();
    w.append(EventPayload::StepObservation {
        step_index: 0,
        output_name: STEP_SCREENSHOT_OBSERVATION.into(),
        value: ObservationValue::Blob {
            blob_sha256: shot.0.clone(),
        },
    })
    .await
    .unwrap();
    let video = w
        .write_blob(&[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0])
        .await
        .unwrap();
    w.append(EventPayload::StepObservation {
        step_index: 0,
        output_name: "capture/video".into(),
        value: ObservationValue::Blob {
            blob_sha256: video.0.clone(),
        },
    })
    .await
    .unwrap();
    let mut session = SessionEvidence::v1(25.0);
    session.steps.push(SessionStep {
        step_index: 0,
        started_ms: 1.0,
        finished_ms: 20.0,
        screenshot_sha256: Some(shot.0.clone()),
        screenshot_ms: Some(20.0),
        frame_error: None,
    });
    session.network.push(SessionNetworkEntry {
        method: "GET".into(),
        url: "http://sut/".into(),
        status: 200,
        started_ms: 2.0,
        duration_ms: 5.0,
        wait_ms: 4.0,
        receive_ms: 1.0,
    });
    session.performance.push(SessionPerformanceObservation {
        kind: "navigation".into(),
        name: "document".into(),
        started_ms: 1.5,
        duration_ms: 6.0,
        value: None,
        unit: None,
    });
    session.video = Some(SessionVideo {
        started_ms: 0.0,
        finished_ms: 25.0,
        blob_sha256: video.0.clone(),
    });
    let session_blob = w
        .write_blob(&serde_json::to_vec(&session).unwrap())
        .await
        .unwrap();
    w.append(EventPayload::StepObservation {
        step_index: 0,
        output_name: SESSION_EVIDENCE_OBSERVATION.into(),
        value: ObservationValue::Blob {
            blob_sha256: session_blob.0.clone(),
        },
    })
    .await
    .unwrap();
    w.append(EventPayload::AssertionEvaluated {
        check_id: "AC-1.1".into(),
        assertion_index: 0,
        state: VerdictState::Pass,
        detail: None,
        expr: Some("true".into()),
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
        verdict: Some(VerdictState::Pass),
    })
    .await
    .unwrap();
    w.finish().await.unwrap();
    (shot.0, video.0)
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
        verdict: Some(VerdictState::Fail),
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
        verdict: Some(VerdictState::Inconclusive(
            InconclusiveCause::EnvironmentError,
        )),
    })
    .await
    .unwrap();
    w.finish().await.unwrap();
}

/// An in-progress run: a step has started and no terminal marker exists.
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
    // Fresh heartbeat age keeps this unterminated run `running`.
}
