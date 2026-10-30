//! Real SQLite actions exercise gating without opening Tier 2 authoring.
use std::{collections::BTreeMap, sync::Arc};

use duhem_dashboard::EvidenceReader;
use duhem_evidence::{EventPayload, EvidenceWriter, SqliteStore, StepOutcome, Trace, replay};
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use serde_json::json;

fn definition(condition: &str) -> VerificationDefinition {
    VerificationDefinition::from_yaml_str(&format!(
        r#"
verification: shrunken-claims
setup:
  - id: count_rows
    uses: db/query
    with: {{ connection: 'sqlite::memory:', sql: 'select 1 where 0' }}
  - uses: db/observe
    if: $setup.count_rows.outputs.row_count > 0
    with: {{ connection: 'sqlite::memory:', sql: 'select 1', until: {{ row_count: 1 }} }}
criteria:
  - id: AC-1
    description: Conditional claims remain legible
    checks:
      - id: AC-1.1
        steps:
          - uses: db/observe
            if: {condition}
            with: {{ connection: 'sqlite::memory:', sql: 'select 1', until: {{ row_count: 1 }} }}
        assertions: ["true"]
"#
    ))
    .unwrap()
}

#[tokio::test]
async fn lifecycle_conditions_record_actual_operands_but_only_value_check_gates_count() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("evidence.db"))
            .await
            .unwrap(),
    );
    let reader = EvidenceReader::new(store.clone());
    for (condition, expected) in [
        ("always", 0),
        ("failure", 0),
        ("$setup.count_rows.outputs.row_count > 0", 1),
    ] {
        let outcome = Engine::new()
            .with_store(store.clone())
            .run_with_metadata(&definition(condition), BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(outcome.verdict.state, VerdictState::Pass);
        assert_eq!(outcome.gated_checks.values().sum::<u32>(), expected);
        let trace = Trace::from_store(store.as_ref(), &outcome.run_id)
            .await
            .unwrap();
        assert!(trace.events().iter().any(|event| matches!(&event.payload,
            EventPayload::SetupStepFinished { outcome: StepOutcome::Skipped {
                condition: Some(condition), operands: Some(operands), .. }, .. }
                if condition == "$setup.count_rows.outputs.row_count > 0"
                    && operands == &BTreeMap::from([("$setup.count_rows.outputs.row_count".into(), json!(0))])
        )));
        let check = reader
            .check_detail(&outcome.run_id, "AC-1", "AC-1.1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(check.gated_judging_steps, expected);
        let envelope = serde_json::to_value(
            reader
                .failure_envelope(&outcome.run_id)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        if expected == 0 {
            assert!(envelope.get("gated_checks").is_none());
            assert!(
                serde_json::to_value(check)
                    .unwrap()
                    .get("gated_judging_steps")
                    .is_none()
            );
        } else {
            assert_eq!(envelope["gated_checks"][0]["gated_judging_steps"], 1);
            assert_eq!(envelope["failing"], json!([]));
        }
    }
}

#[tokio::test]
async fn value_gated_trace_renders_differently_and_replays_identically_without_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("evidence.db"))
            .await
            .unwrap(),
    );
    let reader = EvidenceReader::new(store.clone());
    let def = definition("$setup.count_rows.outputs.row_count > 0");
    // Exercise the internal evidence path; the public Tier 2 authoring
    // boundary remains closed and is pinned here.
    assert!(
        duhem_schema::validate(&def)
            .unwrap_err()
            .iter()
            .any(|error| error
                .to_string()
                .contains("value-based conditions are not yet available"))
    );
    let outcome = Engine::new()
        .with_store(store.clone())
        .run_with_metadata(&def, BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    let trace = Trace::from_store(store.as_ref(), &outcome.run_id)
        .await
        .unwrap();
    assert!(trace.events().iter().any(|event| matches!(&event.payload,
        EventPayload::StepFinished { outcome: StepOutcome::Skipped {
            condition: Some(condition), operands: Some(operands), .. }, .. }
            if condition == "$setup.count_rows.outputs.row_count > 0"
                && operands == &BTreeMap::from([("$setup.count_rows.outputs.row_count".into(), json!(0))])
    )));
    let mut events = trace.into_events();
    let annotated = Trace::from_events(events.clone()).unwrap();
    let before = serde_json::to_vec(&replay(&annotated).unwrap().run).unwrap();
    for event in &mut events {
        match &mut event.payload {
            EventPayload::StepFinished {
                outcome:
                    StepOutcome::Skipped {
                        condition,
                        operands,
                        ..
                    },
                ..
            }
            | EventPayload::SetupStepFinished {
                outcome:
                    StepOutcome::Skipped {
                        condition,
                        operands,
                        ..
                    },
                ..
            } => {
                *condition = None;
                *operands = None;
            }
            EventPayload::CheckFinished {
                gated_judging_steps,
                ..
            } => *gated_judging_steps = 0,
            _ => {}
        }
    }
    let legacy = Trace::from_events(events).unwrap();
    assert_eq!(
        before,
        serde_json::to_vec(&replay(&legacy).unwrap().run).unwrap()
    );

    for (id, trace) in [("annotated", &annotated), ("legacy", &legacy)] {
        let mut writer =
            EvidenceWriter::begin(store.clone(), id, "shrunken-claims", BTreeMap::new())
                .await
                .unwrap();
        for event in trace.events() {
            writer.append(event.payload.clone()).await.unwrap();
        }
        writer.finish().await.unwrap();
    }
    let annotated_check = reader
        .check_detail("annotated", "AC-1", "AC-1.1")
        .await
        .unwrap()
        .unwrap();
    let legacy_check = reader
        .check_detail("legacy", "AC-1", "AC-1.1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(annotated_check.verdict, legacy_check.verdict);
    let annotated_report = reader.run_detail("annotated").await.unwrap().unwrap();
    let legacy_report = reader.run_detail("legacy").await.unwrap().unwrap();
    assert_ne!(
        serde_json::to_value(annotated_report.criteria).unwrap(),
        serde_json::to_value(legacy_report.criteria).unwrap()
    );
    let mut annotated_envelope =
        serde_json::to_value(reader.failure_envelope("annotated").await.unwrap().unwrap()).unwrap();
    let mut legacy_envelope =
        serde_json::to_value(reader.failure_envelope("legacy").await.unwrap().unwrap()).unwrap();
    annotated_envelope.as_object_mut().unwrap().remove("run_id");
    legacy_envelope.as_object_mut().unwrap().remove("run_id");
    assert_ne!(
        annotated_envelope, legacy_envelope,
        "agent rendering must expose the smaller claim set even on pass"
    );
    assert!(legacy_envelope.get("gated_checks").is_none());
    assert_eq!(
        annotated_envelope["gated_checks"][0]["gated_judging_steps"],
        1
    );
}

#[tokio::test]
async fn condition_operands_follow_the_existing_secret_masking_boundary() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("evidence.db"))
            .await
            .unwrap(),
    );
    let def = VerificationDefinition::from_yaml_str(
        r#"
verification: masked-condition
inputs:
  token: { type: string, secret: true }
setup:
  - uses: cli/invoke
    if: $inputs.token == 'not-the-token'
    with: { command: [echo, unreachable] }
criteria:
  - id: AC-1
    description: Secret operands remain masked
    checks:
      - id: C
        steps:
          - id: login
            uses: db/query
            with:
              connection: 'sqlite::memory:'
              sql: "select 'session:' || ?1 as session, upper(?1) as transformed"
              params: [$inputs.token]
          - uses: db/observe
            if: $steps.login.outputs.rows[0].session == 'not-the-session'
            with: { connection: 'sqlite::memory:', sql: 'select 1', until: { row_count: 1 } }
          - uses: db/observe
            if: $steps.login.outputs.rows[0].transformed == 'not-the-session'
            with: { connection: 'sqlite::memory:', sql: 'select 1', until: { row_count: 1 } }
        assertions: ["true"]
"#,
    )
    .unwrap();
    let secret = "sensitive-condition-operand";
    let outcome = Engine::new()
        .with_store(store.clone())
        .run_with_metadata(&def, BTreeMap::from([("token".into(), json!(secret))]))
        .await
        .unwrap();
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    let trace = Trace::from_store(store.as_ref(), &outcome.run_id)
        .await
        .unwrap();
    let encoded = serde_json::to_string(trace.events()).unwrap();
    assert!(!encoded.contains(secret));
    assert!(trace.events().iter().any(|event| matches!(&event.payload,
        EventPayload::SetupStepFinished { outcome: StepOutcome::Skipped { operands: Some(values), .. }, .. }
        if values.get("$inputs.token") == Some(&json!("[redacted:token]"))
    )));
    // The session is computed by a real action and its output path is not
    // declared secret. Substring masking still applies to its operand.
    assert!(trace.events().iter().any(|event| matches!(&event.payload,
        EventPayload::StepFinished { outcome: StepOutcome::Skipped { operands: Some(values), .. }, .. }
        if values.get("$steps.login.outputs.rows.0.session")
            == Some(&json!("session:[redacted:token]"))
    )));
    // The existing boundary is not taint tracking: application-specific
    // transformations outside the registered encodings remain visible.
    assert!(trace.events().iter().any(|event| matches!(&event.payload,
        EventPayload::StepFinished { outcome: StepOutcome::Skipped { operands: Some(values), .. }, .. }
        if values.get("$steps.login.outputs.rows.0.transformed")
            == Some(&json!(secret.to_uppercase()))
    )));
}
