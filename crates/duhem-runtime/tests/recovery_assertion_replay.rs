//! Recovery assertions remain observable without changing the body judgment (#683).
use std::{collections::BTreeMap, sync::Arc};

use duhem_evidence::{
    EventPayload, RunBundle, RunStatus, SqliteStore, Store, Trace, replay, replay_aborted,
};
use duhem_judge::{InconclusiveCause, VerdictState};
use duhem_runtime::Engine;

#[tokio::test]
async fn diagnostic_recovery_assertion_survives_bundle_and_ingest() {
    let tmp = tempfile::tempdir().unwrap();
    let source = Arc::new(
        SqliteStore::open(tmp.path().join("source.db"))
            .await
            .unwrap(),
    );
    let target = SqliteStore::open(tmp.path().join("target.db"))
        .await
        .unwrap();
    let yaml = include_str!("../../../verifications/duhem-cli/fixtures/step-groups-gate.yml");
    let path = tmp.path().join("fixture.yml");
    std::fs::write(&path, yaml).unwrap();
    let duhem_schema::Loaded::Leaf {
        definition: def, ..
    } = duhem_schema::load(&path).unwrap()
    else {
        panic!("expected leaf");
    };
    duhem_schema::validate_with_action_catalog(&def, &|uses| {
        duhem_actions::contract_for(uses).map(|contract| {
            contract
                .outputs
                .iter()
                .map(|output| output.to_string())
                .collect()
        })
    })
    .unwrap();
    let outcome = Engine::new()
        .with_store(source.clone())
        .run_with_metadata(&def, BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        outcome.verdict.criteria[0].checks[0].state,
        VerdictState::Inconclusive(InconclusiveCause::MissingObservation)
    );
    let bundle = RunBundle::from_store(source.as_ref(), &outcome.run_id)
        .await
        .unwrap();
    let bundle: RunBundle = serde_json::from_slice(&bundle.wire_bytes().unwrap()).unwrap();
    assert!(bundle.events.iter().any(|event| matches!(
        event.payload,
        EventPayload::AssertionEvaluated {
            diagnostic: true,
            state: VerdictState::Fail,
            ..
        }
    )));
    let trace = Trace::from_events(bundle.events.clone()).unwrap();
    assert_eq!(replay(&trace).unwrap().run, outcome.verdict);
    target
        .ingest_bundle(&bundle, &bundle.content_hash().unwrap())
        .await
        .unwrap();

    // Removing the recorded distinction must expose the original divergence.
    let mut legacy = serde_json::to_value(&bundle).unwrap();
    for event in legacy["events"].as_array_mut().unwrap() {
        event.as_object_mut().unwrap().remove("diagnostic");
    }
    let legacy: RunBundle = serde_json::from_value(legacy).unwrap();
    let error = replay(&Trace::from_events(legacy.events).unwrap()).unwrap_err();
    assert!(error.to_string().contains("check AC-1.1"));
    assert!(error.to_string().contains("recomputed fail"));
}

#[tokio::test]
async fn engine_error_seals_cleanup_example_and_ingests_without_a_verdict() {
    let tmp = tempfile::tempdir().unwrap();
    let source = Arc::new(
        SqliteStore::open(tmp.path().join("source.db"))
            .await
            .unwrap(),
    );
    let target = Arc::new(
        SqliteStore::open(tmp.path().join("target.db"))
            .await
            .unwrap(),
    );
    // The actual AC-15.1 fixture, preceded by a completed criterion so abort
    // validation must check earlier judgments as well as allow an open scope.
    let yaml = include_str!("../../../verifications/duhem-cli/fixtures/cleanup/duhem.yml")
        .replace("criteria:\n", "criteria:\n  - id: AC-completed\n    description: Earlier judgments survive an engine abort.\n    checks:\n      - id: AC-completed.1\n        steps: []\n        assertions: [\"true\"]\n");
    let path = tmp.path().join("fixture.yml");
    std::fs::write(&path, yaml).unwrap();
    let duhem_schema::Loaded::Leaf {
        definition: def, ..
    } = duhem_schema::load(&path).unwrap()
    else {
        panic!("expected leaf");
    };
    let cleanup_dir = tmp.path().join("cleanup");
    let error = Engine::new()
        .with_store(source.clone())
        .run_with_metadata(
            &def,
            BTreeMap::from([("cleanup_dir".into(), serde_json::json!(cleanup_dir))]),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error, duhem_runtime::EngineError::UnresolvedReference { reference, .. }
        if reference.contains("gated_source"))
    );
    assert!(
        !cleanup_dir.exists(),
        "both cleanup layers must finish before sealing"
    );
    let runs = source.list_runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.status, RunStatus::Aborted);
    assert_eq!(run.verdict, None);
    let bundle = RunBundle::from_store(source.as_ref(), &run.run_id)
        .await
        .unwrap();
    let bundle: RunBundle = serde_json::from_slice(&bundle.wire_bytes().unwrap()).unwrap();
    assert_eq!(
        bundle
            .events
            .iter()
            .filter(|event| matches!(
                event.payload,
                EventPayload::RunFinished { .. } | EventPayload::RunAborted { .. }
            ))
            .count(),
        1
    );
    assert!(matches!(&bundle.events.last().unwrap().payload,
        EventPayload::RunAborted { signal, error: Some(recorded) }
        if signal.is_empty() && recorded == &error.to_string()));
    let trace = Trace::from_events(bundle.events.clone()).unwrap();
    replay_aborted(&trace).unwrap();
    assert_eq!(
        replay(&trace).unwrap_err(),
        duhem_evidence::ReplayError::MissingRunFinished
    );

    let app = duhem_dashboard::ingest::ingest_router(target.clone(), None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let response = reqwest::Client::new()
        .post(format!("http://{address}/api/ingest"))
        .header("content-type", "application/json")
        .header("x-duhem-bundle-version", "1")
        .header("x-duhem-content-hash", bundle.content_hash().unwrap())
        .body(bundle.wire_bytes().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    let received = target.get_run(&run.run_id).await.unwrap().unwrap();
    assert_eq!(received.status, RunStatus::Aborted);
    assert_eq!(received.verdict, None);
    let detail = duhem_dashboard::EvidenceReader::new(target.clone())
        .run_detail(&run.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.error.as_deref(), Some(error.to_string().as_str()));

    // The original fixture alone has no completed criteria. It is still a
    // valid abort; the validator must not require a fabricated empty verdict.
    let mut open = bundle.clone();
    open.events.retain(|event| !matches!(event.payload,
        EventPayload::AssertionEvaluated { ref check_id, .. } | EventPayload::CheckFinished { ref check_id, .. }
            if check_id == "AC-completed.1") && !matches!(event.payload,
        EventPayload::CriterionFinished { ref criterion_id, .. } if criterion_id == "AC-completed"));
    for (seq, event) in open.events.iter_mut().enumerate() {
        event.seq = seq as u64;
    }
    replay_aborted(&Trace::from_events(open.events).unwrap()).unwrap();
    let mut missing = bundle.events.clone();
    missing.pop();
    assert!(replay_aborted(&Trace::from_events(missing).unwrap()).is_err());
    let mut duplicate = bundle.events.clone();
    let mut terminal = duplicate.last().unwrap().clone();
    terminal.seq += 1;
    duplicate.push(terminal);
    assert!(replay_aborted(&Trace::from_events(duplicate).unwrap()).is_err());
    let mut after_terminal = bundle.events.clone();
    let mut heartbeat = after_terminal.last().unwrap().clone();
    heartbeat.seq += 1;
    heartbeat.payload = EventPayload::RunHeartbeat;
    after_terminal.push(heartbeat);
    assert!(replay_aborted(&Trace::from_events(after_terminal).unwrap()).is_err());

    for criterion in [false, true] {
        let mut tampered = bundle.clone();
        let event = tampered
            .events
            .iter_mut()
            .find(|event| {
                if criterion {
                    matches!(event.payload, EventPayload::CriterionFinished { .. })
                } else {
                    matches!(event.payload, EventPayload::CheckFinished { .. })
                }
            })
            .unwrap();
        match &mut event.payload {
            EventPayload::CheckFinished { verdict, .. }
            | EventPayload::CriterionFinished { verdict, .. } => *verdict = VerdictState::Fail,
            _ => unreachable!(),
        }
        assert!(replay_aborted(&Trace::from_events(tampered.events.clone()).unwrap()).is_err());
        let response = reqwest::Client::new()
            .post(format!("http://{address}/api/ingest"))
            .header("content-type", "application/json")
            .header("x-duhem-bundle-version", "1")
            .header("x-duhem-content-hash", tampered.content_hash().unwrap())
            .body(tampered.wire_bytes().unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }
    server.abort();
}
