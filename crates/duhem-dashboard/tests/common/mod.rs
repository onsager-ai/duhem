//! Shared fixtures: realistic traces written through the production
//! `EvidenceWriter`, so the dashboard tests exercise the same wire
//! format `duhem run` produces.
//!
//! Each integration-test binary compiles this module independently
//! and uses a different subset of the fixtures, so the unused-item
//! lint is suppressed at module level.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;

use duhem_evidence::{
    EventPayload, EvidenceWriter, ObservationValue, StepOutcome, VerdictState, run_started,
};
use duhem_judge::InconclusiveCause;

pub const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

pub fn png_bytes() -> Vec<u8> {
    let mut bytes = PNG_MAGIC.to_vec();
    bytes.extend(std::iter::repeat_n(0u8, 64));
    bytes
}

fn inputs() -> BTreeMap<String, serde_json::Value> {
    let mut m = BTreeMap::new();
    m.insert("workspace_name".into(), serde_json::json!("ws-fixture"));
    m
}

/// A finished, passing run with env + setup + one criterion (AC-1)
/// holding one check (AC-1.1) of two steps; the second step records a
/// PNG blob artifact. Returns the blob's sha-256.
pub fn write_passing_run(run_dir: &Path, definition_path: &str) -> String {
    let mut w = EvidenceWriter::new(run_dir, definition_path).expect("writer");
    w.append(run_started(definition_path, inputs())).unwrap();
    w.append(EventPayload::EnvUpStarted {
        command: "./up.sh".into(),
    })
    .unwrap();
    w.append(EventPayload::EnvUpFinished {
        exit_code: 0,
        duration_ms: 120,
        stdout_blob_sha256: None,
        stderr_blob_sha256: None,
    })
    .unwrap();
    w.append(EventPayload::EnvReady {
        probe_kind: "http".into(),
        ok: true,
        elapsed_ms: 30,
    })
    .unwrap();
    w.append(EventPayload::SetupStarted { step_count: 1 })
        .unwrap();
    w.append(EventPayload::SetupStepStarted {
        step_index: 0,
        uses: "ui/navigate".into(),
        with: BTreeMap::new(),
    })
    .unwrap();
    w.append(EventPayload::SetupStepObservation {
        step_index: 0,
        output_name: "landed_at".into(),
        value: ObservationValue::Inline {
            value: serde_json::json!("http://sut/"),
        },
    })
    .unwrap();
    w.append(EventPayload::SetupStepFinished {
        step_index: 0,
        outcome: StepOutcome::Ok,
    })
    .unwrap();
    w.append(EventPayload::SetupFinished { aborted: false })
        .unwrap();

    w.append(EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: 0,
        uses: "ui/navigate".into(),
        with: BTreeMap::new(),
    })
    .unwrap();
    w.append(EventPayload::StepObservation {
        step_index: 0,
        output_name: "landed_at".into(),
        value: ObservationValue::Inline {
            value: serde_json::json!("http://sut/projects"),
        },
    })
    .unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 0,
        outcome: StepOutcome::Ok,
    })
    .unwrap();

    let sha = w.write_blob(&png_bytes()).unwrap();
    w.append(EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: 1,
        uses: "ui/assert-element".into(),
        with: BTreeMap::new(),
    })
    .unwrap();
    w.append(EventPayload::StepObservation {
        step_index: 1,
        output_name: "screenshot".into(),
        value: ObservationValue::Blob {
            blob_sha256: sha.as_str().to_string(),
        },
    })
    .unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 1,
        outcome: StepOutcome::Ok,
    })
    .unwrap();

    w.append(EventPayload::AssertionEvaluated {
        check_id: "AC-1.1".into(),
        assertion_index: 0,
        state: VerdictState::Pass,
        detail: None,
    })
    .unwrap();
    w.append(EventPayload::CheckFinished {
        check_id: "AC-1.1".into(),
        verdict: VerdictState::Pass,
    })
    .unwrap();
    w.append(EventPayload::CriterionFinished {
        criterion_id: "AC-1".into(),
        verdict: VerdictState::Pass,
    })
    .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Pass,
    })
    .unwrap();
    w.finish().unwrap();
    sha.as_str().to_string()
}

/// A finished failing run: one criterion, one check, one failing
/// assertion.
pub fn write_failing_run(run_dir: &Path, definition_path: &str) {
    let mut w = EvidenceWriter::new(run_dir, definition_path).expect("writer");
    w.append(run_started(definition_path, BTreeMap::new()))
        .unwrap();
    w.append(EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: 0,
        uses: "api/call".into(),
        with: BTreeMap::new(),
    })
    .unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 0,
        outcome: StepOutcome::Ok,
    })
    .unwrap();
    w.append(EventPayload::AssertionEvaluated {
        check_id: "AC-1.1".into(),
        assertion_index: 0,
        state: VerdictState::Fail,
        detail: Some("status 500 != 200".into()),
    })
    .unwrap();
    w.append(EventPayload::CheckFinished {
        check_id: "AC-1.1".into(),
        verdict: VerdictState::Fail,
    })
    .unwrap();
    w.append(EventPayload::CriterionFinished {
        criterion_id: "AC-1".into(),
        verdict: VerdictState::Fail,
    })
    .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Fail,
    })
    .unwrap();
    w.finish().unwrap();
}

/// A setup-aborted run, mirroring the engine's abort path (#20): no
/// checks ran, `setup_finished { aborted: true }`, run verdict
/// `inconclusive:environment_error`.
pub fn write_aborted_run(run_dir: &Path, definition_path: &str) {
    let mut w = EvidenceWriter::new(run_dir, definition_path).expect("writer");
    w.append(run_started(definition_path, BTreeMap::new()))
        .unwrap();
    w.append(EventPayload::SetupStarted { step_count: 1 })
        .unwrap();
    w.append(EventPayload::SetupStepStarted {
        step_index: 0,
        uses: "api/call".into(),
        with: BTreeMap::new(),
    })
    .unwrap();
    w.append(EventPayload::SetupStepFinished {
        step_index: 0,
        outcome: StepOutcome::Error,
    })
    .unwrap();
    w.append(EventPayload::SetupFinished { aborted: true })
        .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
    })
    .unwrap();
    w.finish().unwrap();
}

/// An in-progress run (#84): a step has started, no `run_finished`,
/// and the file ends with a partially-appended line that a lenient
/// reader must ignore.
pub fn write_in_progress_run(run_dir: &Path, definition_path: &str) {
    let mut w = EvidenceWriter::new(run_dir, definition_path).expect("writer");
    w.append(run_started(definition_path, BTreeMap::new()))
        .unwrap();
    w.append(EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: 0,
        uses: "ui/navigate".into(),
        with: BTreeMap::new(),
    })
    .unwrap();
    w.finish().unwrap();
    // Simulate a writer mid-append: a half line with no trailing \n.
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(run_dir.join("trace.jsonl"))
        .unwrap();
    f.write_all(br#"{"seq":2,"ts":"2026-06-10T0"#).unwrap();
    f.flush().unwrap();
}
