//! Recovery assertions remain observable without changing the body judgment (#683).
use std::{collections::BTreeMap, sync::Arc};

use duhem_evidence::{EventPayload, RunBundle, SqliteStore, Trace, replay};
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
