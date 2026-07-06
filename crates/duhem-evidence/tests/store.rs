//! Integration tests for the DB-backed evidence store (#189).
//!
//! Carries forward every intent from the trace-v1 suite (#10) — round
//! trip, crash tolerance, blob threshold, replay, seq monotonicity —
//! and adds the store-level invariants: append-only rows, sealed
//! runs, and the read-only lens handle.

use std::collections::BTreeMap;
use std::sync::Arc;

use duhem_evidence::{
    BLOB_INLINE_THRESHOLD_BYTES, Event, EventPayload, EvidenceWriter, ObservationValue, ReadError,
    ReplayDivergence, ReplayError, SqliteStore, StepOutcome, Store, Trace, VerdictState, replay,
    run_started,
};
use tempfile::TempDir;

async fn open_store() -> (TempDir, Arc<SqliteStore>) {
    let tmp = TempDir::new().unwrap();
    let store = SqliteStore::open(tmp.path().join("duhem.db"))
        .await
        .unwrap();
    (tmp, Arc::new(store))
}

const RUN_ID: &str = "01ABCDEFGHIJKLMNOPQRSTUVWX";

/// Spec's worked-example run, fully written through the writer.
async fn write_worked_example(store: Arc<SqliteStore>) {
    let mut inputs = BTreeMap::new();
    inputs.insert("workspace_name".into(), serde_json::json!("test-ws-018f"));
    let mut w = EvidenceWriter::begin(store, RUN_ID, "create-workspace.yml", inputs.clone())
        .await
        .unwrap();
    w.append(run_started("create-workspace.yml", inputs))
        .await
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
        state: VerdictState::Pass,
        detail: None,
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
}

#[tokio::test]
async fn round_trip_worked_example() {
    let (_tmp, store) = open_store().await;
    write_worked_example(store.clone()).await;

    let trace = Trace::from_store(store.as_ref(), RUN_ID).await.unwrap();
    let events = trace.events();
    assert_eq!(events.len(), 7);
    assert!(matches!(events[0].payload, EventPayload::RunStarted { .. }));
    assert!(matches!(
        events.last().unwrap().payload,
        EventPayload::RunFinished {
            verdict: VerdictState::Pass
        }
    ));

    // The run header (manifest successor) is queryable and complete.
    let run = store.get_run(RUN_ID).await.unwrap().unwrap();
    assert_eq!(run.schema_version, "v1");
    assert_eq!(run.verification, "create-workspace.yml");
    assert_eq!(run.verdict, Some(VerdictState::Pass));
    assert!(run.duration_ms.is_some());
    assert_eq!(
        run.inputs.get("workspace_name"),
        Some(&serde_json::json!("test-ws-018f"))
    );

    // Derived projections folded from the stream.
    let criteria: Vec<(String, String)> =
        sqlx::query_as("SELECT criterion_id, verdict FROM criteria WHERE run_id = ?")
            .bind(RUN_ID)
            .fetch_all(store.pool())
            .await
            .unwrap();
    assert_eq!(criteria, vec![("AC-1".to_string(), "pass".to_string())]);
    let checks: Vec<(String, Option<String>, String)> =
        sqlx::query_as("SELECT check_id, criterion_id, verdict FROM checks WHERE run_id = ?")
            .bind(RUN_ID)
            .fetch_all(store.pool())
            .await
            .unwrap();
    assert_eq!(
        checks,
        vec![(
            "AC-1.1".to_string(),
            Some("AC-1".to_string()),
            "pass".to_string()
        )]
    );
}

#[tokio::test]
async fn dropped_writer_loses_nothing_and_run_stays_unfinished() {
    // The process-crash analog: every append is a committed
    // transaction, so dropping the writer without `finish` loses
    // nothing; the run simply has no verdict row (in-flight).
    let (_tmp, store) = open_store().await;
    {
        let mut w = EvidenceWriter::begin(store.clone(), RUN_ID, "x.yml", BTreeMap::new())
            .await
            .unwrap();
        w.append(run_started("x.yml", BTreeMap::new()))
            .await
            .unwrap();
        for i in 0..9 {
            w.append(EventPayload::AssertionEvaluated {
                check_id: format!("C{i}"),
                assertion_index: 0,
                state: VerdictState::Pass,
                detail: None,
            })
            .await
            .unwrap();
        }
        // No `finish()` — writer dropped here.
    }

    let trace = Trace::from_store(store.as_ref(), RUN_ID).await.unwrap();
    assert_eq!(trace.events().len(), 10);
    for (i, evt) in trace.events().iter().enumerate() {
        assert_eq!(evt.seq, i as u64);
    }
    let run = store.get_run(RUN_ID).await.unwrap().unwrap();
    assert_eq!(run.verdict, None, "no verdict row without run_finished");
}

#[tokio::test]
async fn blob_threshold_inlines_small_and_blobs_large() {
    let (_tmp, store) = open_store().await;
    let mut w = EvidenceWriter::begin(store.clone(), RUN_ID, "x.yml", BTreeMap::new())
        .await
        .unwrap();
    w.append(run_started("x.yml", BTreeMap::new()))
        .await
        .unwrap();

    // 1 KiB string → inlines.
    let small = "a".repeat(1024);
    w.append_observation(0, "small", serde_json::json!(small))
        .await
        .unwrap();
    // 5 KiB string → blobs.
    let large = "b".repeat(5 * 1024);
    w.append_observation(0, "large", serde_json::json!(large))
        .await
        .unwrap();
    w.finish().await.unwrap();

    let trace = Trace::from_store(store.as_ref(), RUN_ID).await.unwrap();
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
                let bytes = store.get_blob(blob_sha256).await.unwrap().unwrap();
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

    assert_eq!(BLOB_INLINE_THRESHOLD_BYTES, 4 * 1024);
}

#[tokio::test]
async fn put_blob_is_idempotent() {
    let (_tmp, store) = open_store().await;
    let a = store.put_blob(b"same bytes").await.unwrap();
    let b = store.put_blob(b"same bytes").await.unwrap();
    assert_eq!(a, b);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artifacts")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn replay_passes_on_recorded_run() {
    let (_tmp, store) = open_store().await;
    write_worked_example(store.clone()).await;
    let trace = Trace::from_store(store.as_ref(), RUN_ID).await.unwrap();
    let replayed = replay(&trace).unwrap();
    assert_eq!(replayed.run.state, VerdictState::Pass);
    assert_eq!(replayed.run.criteria.len(), 1);
    let criterion = &replayed.run.criteria[0];
    assert_eq!(criterion.criterion_id, "AC-1");
    assert_eq!(criterion.checks.len(), 1);
    assert_eq!(criterion.checks[0].check_id, "AC-1.1");
}

#[tokio::test]
async fn replay_divergence_when_assertion_state_flipped() {
    // Hand-construct a stream where `check_finished.verdict = pass`
    // but the only assertion is `fail` — replay must catch it.
    let jsonl = [
        r#"{"seq":0,"ts":"2026-05-08T12:00:00.000Z","kind":"run_started","verification_path":"x.yml","schema_version":"v1"}"#,
        r#"{"seq":1,"ts":"2026-05-08T12:00:00.001Z","kind":"step_started","criterion_id":"AC-1","check_id":"AC-1.1","step_index":0,"uses":"ui/click"}"#,
        r#"{"seq":2,"ts":"2026-05-08T12:00:00.002Z","kind":"step_finished","step_index":0,"outcome":"ok"}"#,
        r#"{"seq":3,"ts":"2026-05-08T12:00:00.003Z","kind":"assertion_evaluated","check_id":"AC-1.1","assertion_index":0,"state":"fail","detail":null}"#,
        r#"{"seq":4,"ts":"2026-05-08T12:00:00.004Z","kind":"check_finished","check_id":"AC-1.1","verdict":"pass"}"#,
        r#"{"seq":5,"ts":"2026-05-08T12:00:00.005Z","kind":"criterion_finished","criterion_id":"AC-1","verdict":"pass"}"#,
        r#"{"seq":6,"ts":"2026-05-08T12:00:00.006Z","kind":"run_finished","verdict":"pass"}"#,
    ]
    .join("\n");

    let trace = Trace::from_jsonl(&jsonl).unwrap();
    match replay(&trace) {
        Err(ReplayError::Divergence(ReplayDivergence::Check {
            check_id,
            recorded,
            recomputed,
        })) => {
            assert_eq!(check_id, "AC-1.1");
            assert_eq!(recorded, VerdictState::Pass);
            assert_eq!(recomputed, VerdictState::Fail);
        }
        other => panic!("expected check divergence, got {other:?}"),
    }
}

#[tokio::test]
async fn seq_monotonicity_enforced_on_read() {
    let jsonl = [
        r#"{"seq":0,"ts":"2026-05-08T12:00:00.000Z","kind":"run_started","verification_path":"x.yml","schema_version":"v1"}"#,
        r#"{"seq":2,"ts":"2026-05-08T12:00:00.001Z","kind":"run_finished","verdict":"pass"}"#,
    ]
    .join("\n");
    match Trace::from_jsonl(&jsonl) {
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

#[tokio::test]
async fn first_event_with_nonzero_seq_reports_no_prev() {
    let jsonl = r#"{"seq":5,"ts":"2026-05-08T12:00:00.000Z","kind":"run_started","verification_path":"x.yml","schema_version":"v1"}"#;
    match Trace::from_jsonl(jsonl) {
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

#[tokio::test]
async fn get_blob_rejects_path_traversal_shaped_digests() {
    let (_tmp, store) = open_store().await;
    assert!(store.get_blob("../etc/passwd").await.is_err());
    assert!(store.get_blob(&"A".repeat(64)).await.is_err());
    assert!(store.get_blob(&"a".repeat(64)).await.unwrap().is_none());
}

#[tokio::test]
async fn unknown_event_kind_is_a_hard_error_on_read() {
    let jsonl = r#"{"seq":0,"ts":"2026-05-08T12:00:00.000Z","kind":"hypothetical_future_kind"}"#;
    match Trace::from_jsonl(jsonl) {
        Err(ReadError::Parse { line, .. }) => assert_eq!(line, 1),
        other => panic!("expected Parse error, got {other:?}"),
    }
}

// ---------------------------------------------------------------
// Store-level invariants (#189)
// ---------------------------------------------------------------

#[tokio::test]
async fn update_and_delete_are_rejected_on_every_table() {
    let (_tmp, store) = open_store().await;
    write_worked_example(store.clone()).await;

    for (table, set_clause) in [
        ("runs", "verification = 'tampered'"),
        ("run_verdicts", "verdict = 'fail'"),
        ("events", "payload = '{}'"),
        ("criteria", "verdict = 'fail'"),
        ("checks", "verdict = 'fail'"),
        ("assertions", "state = 'fail'"),
    ] {
        let update = sqlx::query(&format!("UPDATE {table} SET {set_clause}"))
            .execute(store.pool())
            .await;
        let err = update.expect_err(&format!("UPDATE on {table} must be rejected"));
        assert!(
            err.to_string().contains("append-only"),
            "unexpected error for UPDATE {table}: {err}"
        );
        let delete = sqlx::query(&format!("DELETE FROM {table}"))
            .execute(store.pool())
            .await;
        let err = delete.expect_err(&format!("DELETE on {table} must be rejected"));
        assert!(
            err.to_string().contains("append-only"),
            "unexpected error for DELETE {table}: {err}"
        );
    }
}

#[tokio::test]
async fn a_finished_run_is_sealed_against_further_events() {
    let (_tmp, store) = open_store().await;
    let mut w = EvidenceWriter::begin(store.clone(), RUN_ID, "x.yml", BTreeMap::new())
        .await
        .unwrap();
    w.append(run_started("x.yml", BTreeMap::new()))
        .await
        .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Pass,
    })
    .await
    .unwrap();

    let err = w
        .append(EventPayload::SetupStarted { step_count: 1 })
        .await
        .expect_err("appending after run_finished must fail");
    assert!(
        err.to_string().contains("sealed"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn read_only_handle_cannot_write_but_can_read() {
    let (tmp, store) = open_store().await;
    write_worked_example(store.clone()).await;

    let ro = SqliteStore::open_read_only(tmp.path().join("duhem.db"))
        .await
        .unwrap();
    assert!(ro.is_read_only());

    // Reads work.
    let runs = ro.list_runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, RUN_ID);
    let events = ro.run_events(RUN_ID).await.unwrap();
    assert_eq!(events.len(), 7);

    // Writes fail at the connection level — both raw SQL and the
    // trait surface.
    assert!(
        sqlx::query("INSERT INTO artifacts (sha256, size, bytes) VALUES ('x', 1, x'00')")
            .execute(ro.pool())
            .await
            .is_err()
    );
    assert!(ro.put_blob(b"nope").await.is_err());
}

#[tokio::test]
async fn open_read_only_refuses_missing_store() {
    let tmp = TempDir::new().unwrap();
    assert!(
        SqliteStore::open_read_only(tmp.path().join("absent.db"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn events_after_supports_live_tailing() {
    let (_tmp, store) = open_store().await;
    let mut w = EvidenceWriter::begin(store.clone(), RUN_ID, "x.yml", BTreeMap::new())
        .await
        .unwrap();
    w.append(run_started("x.yml", BTreeMap::new()))
        .await
        .unwrap();
    w.append(EventPayload::SetupStarted { step_count: 1 })
        .await
        .unwrap();

    let all = store.events_after(RUN_ID, -1).await.unwrap();
    assert_eq!(all.len(), 2);
    let tail = store.events_after(RUN_ID, 0).await.unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].seq, 1);
    assert!(store.events_after(RUN_ID, 1).await.unwrap().is_empty());
}

#[tokio::test]
async fn list_runs_orders_most_recent_first() {
    let (_tmp, store) = open_store().await;
    for id in ["01A", "01B", "01C"] {
        let mut w = EvidenceWriter::begin(store.clone(), id, "x.yml", BTreeMap::new())
            .await
            .unwrap();
        w.append(run_started("x.yml", BTreeMap::new()))
            .await
            .unwrap();
        // Distinct started_at millis are not guaranteed here, so the
        // run_id DESC tiebreak keeps ordering deterministic (ULIDs
        // sort chronologically).
    }
    let runs = store.list_runs().await.unwrap();
    let ids: Vec<&str> = runs.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(ids, vec!["01C", "01B", "01A"]);
}
