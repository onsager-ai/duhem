//! Integration tests for the evidence trace v1 (issue #10).
//!
//! Each test covers one bullet from the spec's Test section.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;

use duhem_evidence::{
    AssertionState, BLOB_INLINE_THRESHOLD_BYTES, Event, EventPayload, EvidenceWriter,
    ObservationValue, ReadError, ReplayDivergence, ReplayError, StepOutcome, Trace, Verdict,
    replay, run_started,
};
use tempfile::TempDir;

fn run_dir() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("01ABCDEFGHIJKLMNOPQRSTUVWX");
    (tmp, dir)
}

/// Spec's worked-example trace, fully written and read back.
fn write_worked_example(dir: &std::path::Path) {
    let mut w = EvidenceWriter::new(dir, "create-workspace.yml").unwrap();
    let mut inputs = BTreeMap::new();
    inputs.insert("workspace_name".into(), serde_json::json!("test-ws-018f"));
    w.append(run_started("create-workspace.yml", inputs))
        .unwrap();

    let mut with = BTreeMap::new();
    with.insert("role".into(), serde_json::json!("button"));
    with.insert("name".into(), serde_json::json!("Create"));
    w.append(EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: 0,
        uses: "ui/click".into(),
        with,
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
        state: AssertionState::Pass,
        detail: None,
    })
    .unwrap();

    w.append(EventPayload::CheckFinished {
        check_id: "AC-1.1".into(),
        verdict: Verdict::Pass,
    })
    .unwrap();

    w.append(EventPayload::CriterionFinished {
        criterion_id: "AC-1".into(),
        verdict: Verdict::Pass,
    })
    .unwrap();

    w.append(EventPayload::RunFinished {
        verdict: Verdict::Pass,
    })
    .unwrap();

    w.finish().unwrap();
}

#[test]
fn round_trip_worked_example() {
    let (_tmp, dir) = run_dir();
    write_worked_example(&dir);

    let trace = Trace::open(&dir).unwrap();
    let events = trace.events();
    assert_eq!(events.len(), 7);
    assert!(matches!(events[0].payload, EventPayload::RunStarted { .. }));
    assert!(matches!(
        events.last().unwrap().payload,
        EventPayload::RunFinished {
            verdict: Verdict::Pass
        }
    ));

    // Manifest exists and is parseable.
    let manifest_bytes = std::fs::read(dir.join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["schema_version"], "v1");
    assert_eq!(manifest["definition_path"], "create-workspace.yml");
}

#[test]
fn crash_recovery_leaves_no_half_line() {
    // Open writer, append 10 events, drop without `finish` — the
    // process-crash analog at the API level.
    let (_tmp, dir) = run_dir();
    {
        let mut w = EvidenceWriter::new(&dir, "x.yml").unwrap();
        w.append(run_started("x.yml", BTreeMap::new())).unwrap();
        for i in 0..9 {
            w.append(EventPayload::AssertionEvaluated {
                check_id: format!("C{i}"),
                assertion_index: 0,
                state: AssertionState::Pass,
                detail: None,
            })
            .unwrap();
        }
        // No `finish()` call — writer goes out of scope here.
    }

    // The first 10 events parse cleanly, and the file ends on `\n`.
    let bytes = std::fs::read(dir.join("trace.jsonl")).unwrap();
    assert!(!bytes.is_empty());
    assert_eq!(*bytes.last().unwrap(), b'\n', "trace must end on \\n");

    let trace = Trace::open(&dir).unwrap();
    assert_eq!(trace.events().len(), 10);
    for (i, evt) in trace.events().iter().enumerate() {
        assert_eq!(evt.seq, i as u64);
    }
}

#[test]
fn blob_threshold_inlines_small_and_blobs_large() {
    let (_tmp, dir) = run_dir();
    let mut w = EvidenceWriter::new(&dir, "x.yml").unwrap();
    w.append(run_started("x.yml", BTreeMap::new())).unwrap();

    // 1 KiB string → inlines.
    let small = "a".repeat(1024);
    w.append_observation(0, "small", serde_json::json!(small))
        .unwrap();

    // 5 KiB string → blobs.
    let large = "b".repeat(5 * 1024);
    w.append_observation(0, "large", serde_json::json!(large))
        .unwrap();

    w.finish().unwrap();

    let trace = Trace::open(&dir).unwrap();
    let observations: Vec<&Event> = trace
        .events()
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::StepObservation { .. }))
        .collect();
    assert_eq!(observations.len(), 2);

    match &observations[0].payload {
        EventPayload::StepObservation { value, .. } => {
            assert!(matches!(value, ObservationValue::Inline { .. }));
        }
        _ => unreachable!(),
    }
    match &observations[1].payload {
        EventPayload::StepObservation { value, .. } => match value {
            ObservationValue::Blob { blob_sha256 } => {
                let bytes = trace.read_blob(blob_sha256).unwrap();
                // The blob holds the serialized JSON value, not the
                // raw string (the writer hashes the serialization so
                // identical values dedupe).
                let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(v.as_str().unwrap().len(), 5 * 1024);
            }
            _ => panic!("expected blob"),
        },
        _ => unreachable!(),
    }

    // Sanity check on the constant — guarantees the test stays
    // meaningful if the threshold ever shifts.
    assert_eq!(BLOB_INLINE_THRESHOLD_BYTES, 4 * 1024);
}

#[test]
fn replay_passes_on_recorded_trace() {
    let (_tmp, dir) = run_dir();
    write_worked_example(&dir);
    let trace = Trace::open(&dir).unwrap();
    let v = replay(&trace).unwrap();
    assert_eq!(v.run, Verdict::Pass);
    assert_eq!(v.checks.get("AC-1.1"), Some(&Verdict::Pass));
    assert_eq!(v.criteria.get("AC-1"), Some(&Verdict::Pass));
}

#[test]
fn replay_divergence_when_assertion_state_flipped() {
    // Hand-construct a trace where `check_finished.verdict = pass`
    // but the only assertion is `fail` — replay must catch it.
    let (_tmp, dir) = run_dir();
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let lines = vec![
        r#"{"seq":0,"ts":"2026-05-08T12:00:00.000Z","kind":"run_started","verification_path":"x.yml","schema_version":"v1"}"#,
        r#"{"seq":1,"ts":"2026-05-08T12:00:00.001Z","kind":"step_started","criterion_id":"AC-1","check_id":"AC-1.1","step_index":0,"uses":"ui/click"}"#,
        r#"{"seq":2,"ts":"2026-05-08T12:00:00.002Z","kind":"step_finished","step_index":0,"outcome":"ok"}"#,
        r#"{"seq":3,"ts":"2026-05-08T12:00:00.003Z","kind":"assertion_evaluated","check_id":"AC-1.1","assertion_index":0,"state":"fail","detail":null}"#,
        r#"{"seq":4,"ts":"2026-05-08T12:00:00.004Z","kind":"check_finished","check_id":"AC-1.1","verdict":"pass"}"#,
        r#"{"seq":5,"ts":"2026-05-08T12:00:00.005Z","kind":"criterion_finished","criterion_id":"AC-1","verdict":"pass"}"#,
        r#"{"seq":6,"ts":"2026-05-08T12:00:00.006Z","kind":"run_finished","verdict":"pass"}"#,
    ];
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(dir.join("trace.jsonl"))
        .unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
    drop(f);

    let trace = Trace::open(&dir).unwrap();
    match replay(&trace) {
        Err(ReplayError::Divergence(ReplayDivergence::Check {
            check_id,
            recorded,
            recomputed,
        })) => {
            assert_eq!(check_id, "AC-1.1");
            assert_eq!(recorded, Verdict::Pass);
            assert_eq!(recomputed, Verdict::Fail);
        }
        other => panic!("expected check divergence, got {other:?}"),
    }
}

#[test]
fn seq_monotonicity_enforced_on_read() {
    let (_tmp, dir) = run_dir();
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    // seq jumps from 0 → 2 (gap) — must be a hard error.
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(dir.join("trace.jsonl"))
        .unwrap();
    writeln!(
        f,
        r#"{{"seq":0,"ts":"2026-05-08T12:00:00.000Z","kind":"run_started","verification_path":"x.yml","schema_version":"v1"}}"#
    )
    .unwrap();
    writeln!(
        f,
        r#"{{"seq":2,"ts":"2026-05-08T12:00:00.001Z","kind":"run_finished","verdict":"pass"}}"#
    )
    .unwrap();
    drop(f);

    match Trace::open(&dir) {
        Err(ReadError::SeqNotMonotonic {
            prev,
            expected,
            got,
            ..
        }) => {
            assert_eq!(prev, Some(0));
            assert_eq!(expected, 1);
            assert_eq!(got, 2);
        }
        other => panic!("expected SeqNotMonotonic, got {other:?}"),
    }
}

#[test]
fn first_event_with_nonzero_seq_reports_no_prev() {
    let (_tmp, dir) = run_dir();
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(dir.join("trace.jsonl"))
        .unwrap();
    writeln!(
        f,
        r#"{{"seq":5,"ts":"2026-05-08T12:00:00.000Z","kind":"run_started","verification_path":"x.yml","schema_version":"v1"}}"#
    )
    .unwrap();
    drop(f);

    match Trace::open(&dir) {
        Err(ReadError::SeqNotMonotonic {
            prev,
            expected,
            got,
            ..
        }) => {
            assert_eq!(prev, None);
            assert_eq!(expected, 0);
            assert_eq!(got, 5);
        }
        other => panic!("expected SeqNotMonotonic with prev=None, got {other:?}"),
    }
}

#[test]
fn read_blob_rejects_path_traversal() {
    let (_tmp, dir) = run_dir();
    write_worked_example(&dir);
    let trace = Trace::open(&dir).unwrap();
    match trace.read_blob("../etc/passwd") {
        Err(ReadError::BadBlobDigest(s)) => assert_eq!(s, "../etc/passwd"),
        other => panic!("expected BadBlobDigest, got {other:?}"),
    }
    // Also rejects uppercase-hex (writer always emits lowercase).
    match trace.read_blob(&"A".repeat(64)) {
        Err(ReadError::BadBlobDigest(_)) => {}
        other => panic!("expected BadBlobDigest, got {other:?}"),
    }
}

#[test]
fn unknown_event_kind_is_a_hard_error_on_read() {
    let (_tmp, dir) = run_dir();
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(dir.join("trace.jsonl"))
        .unwrap();
    writeln!(
        f,
        r#"{{"seq":0,"ts":"2026-05-08T12:00:00.000Z","kind":"hypothetical_future_kind"}}"#
    )
    .unwrap();
    drop(f);

    match Trace::open(&dir) {
        Err(ReadError::Parse { line, .. }) => assert_eq!(line, 1),
        other => panic!("expected Parse error, got {other:?}"),
    }
}
