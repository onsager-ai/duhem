//! #440 Tier 2 through real SQLite actions: an unresolvable check-step
//! condition (R2) and reads of a value-gated step's outputs (R4) make
//! the check inconclusive instead of aborting the run or passing.
use std::{collections::BTreeMap, sync::Arc};

use duhem_evidence::{EventPayload, SqliteStore, StepOutcome, Trace};
use duhem_judge::{InconclusiveCause, VerdictState};
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;

const MISSING: VerdictState = VerdictState::Inconclusive(InconclusiveCause::MissingObservation);

/// AC-2 is an ordinary passing check, proving other criteria still
/// record their own outcomes. `check_body` is the AC-1.1 body.
async fn run(check_body: &str) -> (duhem_runtime::RunOutcome, Trace) {
    let def = VerificationDefinition::from_yaml_str(&format!(
        r#"
verification: check-conditions
criteria:
  - id: AC-1
    description: Subject
    checks:
      - id: AC-1.1
{check_body}
  - id: AC-2
    description: Unaffected
    checks:
      - id: AC-2.1
        steps:
          - id: ok
            uses: db/query
            with: {{ connection: 'sqlite::memory:', sql: 'select 1 as v' }}
        assertions: ["$steps.ok.outputs.row_count == 1"]
"#
    ))
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("evidence.db"))
            .await
            .unwrap(),
    );
    let outcome = Engine::new()
        .with_store(store.clone())
        .run_with_metadata(&def, BTreeMap::new())
        .await
        .expect("an unresolvable check condition must not abort the run");
    let trace = Trace::from_store(store.as_ref(), &outcome.run_id)
        .await
        .unwrap();
    (outcome, trace)
}

const EMPTY_PROBE: &str = r#"          - id: probe
            uses: db/query
            with: { connection: 'sqlite::memory:', sql: 'select 1 as v where 0' }
"#;

fn guarded(condition: &str, id: &str) -> String {
    format!(
        "          - id: {id}\n            uses: db/query\n            if: {condition}\n            with: {{ connection: 'sqlite::memory:', sql: 'select 1 as v' }}\n"
    )
}

#[tokio::test]
async fn unresolvable_condition_makes_only_its_check_inconclusive() {
    let body = format!(
        "        steps:\n{EMPTY_PROBE}{}        assertions: [\"true\"]",
        guarded("$steps.probe.outputs.rows[0].v > 0", "guarded")
    );
    let (outcome, trace) = run(&body).await;
    let checks = |i: usize| outcome.verdict.criteria[i].checks[0].state;
    assert_eq!(checks(0), MISSING);
    assert_eq!(checks(1), VerdictState::Pass);
    assert!(trace.events().iter().any(|event| matches!(&event.payload,
        EventPayload::StepFinished { outcome: StepOutcome::Error, detail: Some(detail), .. }
            if detail.contains("step `guarded`")
                && detail.contains("$steps.probe.outputs.rows[0].v > 0")
    )));
}

#[tokio::test]
async fn assertion_reading_a_gated_step_is_inconclusive_not_omitted() {
    let body = format!(
        "        steps:\n{EMPTY_PROBE}{}        assertions: [\"$steps.guarded.outputs.row_count == 1\"]",
        guarded("$steps.probe.outputs.row_count > 0", "guarded")
    );
    let (outcome, _) = run(&body).await;
    assert_eq!(outcome.verdict.criteria[0].checks[0].state, MISSING);
    assert_eq!(
        outcome.verdict.criteria[1].checks[0].state,
        VerdictState::Pass
    );
}

#[tokio::test]
async fn later_condition_reading_a_gated_step_is_inconclusive() {
    let body = format!(
        "        steps:\n{EMPTY_PROBE}{}{}        assertions: [\"true\"]",
        guarded("$steps.probe.outputs.row_count > 0", "first"),
        guarded("$steps.first.outputs.row_count == 1", "second"),
    );
    let (outcome, _) = run(&body).await;
    assert_eq!(outcome.verdict.criteria[0].checks[0].state, MISSING);
}
