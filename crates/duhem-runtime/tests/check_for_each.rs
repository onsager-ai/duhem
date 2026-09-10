//! Tier 2 through real SQLite actions, validation, persisted evidence and replay.
use duhem_evidence::{EventPayload, SqliteStore, Trace, replay};
use duhem_judge::{InconclusiveCause, VerdictState};
use duhem_runtime::Engine;

use std::sync::Arc;

const EXAMPLE: &str = include_str!("../../../verifications/for-each-rows-example/verification.yml");

async fn run(yaml: &str) -> (VerdictState, Trace) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("verification.yml");
    std::fs::write(&path, yaml).unwrap();
    let duhem_schema::Loaded::Leaf {
        definition: def, ..
    } = duhem_schema::load(&path).unwrap()
    else {
        panic!("expected leaf")
    };
    let inputs = def
        .inputs
        .iter()
        .filter_map(|(name, decl)| {
            decl.default
                .as_ref()
                .map(|v| (name.clone(), serde_json::to_value(v).unwrap()))
        })
        .collect();
    duhem_schema::validate_with_action_catalog(&def, &|uses| {
        duhem_actions::contract_for(uses).map(|c| c.outputs.iter().map(|o| o.to_string()).collect())
    })
    .unwrap();
    let store = Arc::new(
        SqliteStore::open(dir.path().join("evidence.db"))
            .await
            .unwrap(),
    );
    let result = Engine::new()
        .with_store(store.clone())
        .run_with_metadata(&def, inputs)
        .await
        .unwrap();
    let trace = Trace::from_store(store.as_ref(), &result.run_id)
        .await
        .unwrap();
    assert_eq!(replay(&trace).unwrap().run.state, result.verdict.state);
    (result.verdict.state, trace)
}

#[tokio::test]
async fn one_failing_row_preserves_all_n_times_m_assertions() {
    let (state, trace) = run(&EXAMPLE.replace("quantity: 2", "quantity: 0")).await;
    assert_eq!(state, VerdictState::Fail);
    let assertions: Vec<_> = trace
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::AssertionEvaluated {
                assertion_index,
                iteration,
                state,
                ..
            } => Some((*iteration, *assertion_index, *state)),
            _ => None,
        })
        .collect();
    assert_eq!(
        assertions,
        vec![
            (Some(0), 0, VerdictState::Pass),
            (Some(0), 1, VerdictState::Pass),
            (Some(1), 0, VerdictState::Fail),
            (Some(1), 1, VerdictState::Pass),
            (Some(2), 0, VerdictState::Pass),
            (Some(2), 1, VerdictState::Pass),
        ]
    );
    let iterations: Vec<_> = trace
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::StepStarted {
                flow: Some(flow), ..
            } => flow.iteration,
            _ => None,
        })
        .collect();
    assert_eq!(iterations, vec![0, 1, 2]);
}

#[tokio::test]
async fn empty_array_has_no_assertions_and_is_empty_aggregation() {
    let empty = EXAMPLE.replace(
        "default:\n      - {quantity: 1}\n      - {quantity: 2}\n      - {quantity: 3}",
        "default: []",
    );
    let (state, trace) = run(&empty).await;
    assert_eq!(
        state,
        VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation)
    );
    assert!(
        !trace
            .events()
            .iter()
            .any(|e| matches!(e.payload, EventPayload::AssertionEvaluated { .. }))
    );
}

#[tokio::test]
async fn worked_example_passes_and_max_is_a_hard_ceiling() {
    assert_eq!(run(EXAMPLE).await.0, VerdictState::Pass);
    let (state, trace) = run(&EXAMPLE.replace("max: 10", "max: 2")).await;
    assert_eq!(state, VerdictState::Fail);
    assert!(
        !trace
            .events()
            .iter()
            .any(|e| matches!(e.payload, EventPayload::StepStarted { .. }))
    );
}

#[tokio::test]
async fn flow_catalog_loop_projects_each_iterations_output() {
    let yaml = r#"
verification: flow rows
inputs:
  rows: {type: array, default: [1, 0, 3]}
flows:
  inspect_rows:
    params:
      rows: {type: array}
    steps:
      - for_each: $params.rows
        max: 5
        as: row
        steps:
          - id: inspect
            uses: db/query
            with:
              connection: 'sqlite::memory:'
              sql: 'select ? as quantity'
              params: [$row]
    outputs:
      rows: $steps.inspect.outputs.rows
criteria:
  - id: AC-1
    description: All rows are positive
    checks:
      - id: AC-1.1
        steps:
          - id: batch
            call: inspect_rows
            with: {rows: $inputs.rows}
        assertions:
          - $steps.batch.outputs.rows[0].quantity > 0
"#;
    let (state, trace) = run(yaml).await;
    assert_eq!(state, VerdictState::Fail);
    let outcomes: Vec<_> = trace
        .events()
        .iter()
        .filter_map(|e| match e.payload {
            EventPayload::AssertionEvaluated {
                iteration, state, ..
            } => Some((iteration, state)),
            _ => None,
        })
        .collect();
    assert_eq!(
        outcomes,
        vec![
            (Some(0), VerdictState::Pass),
            (Some(1), VerdictState::Fail),
            (Some(2), VerdictState::Pass)
        ]
    );
}

#[tokio::test]
async fn call_and_uses_bodies_keep_implicit_judgments_per_iteration() {
    for body in [
        "uses: db/observe\n            with: {connection: 'sqlite::memory:', sql: 'select 1 as n where ? > 0', params: [$row], timeout: 20ms, until: {row_count: 1}}",
        "call: observe\n            with: {row: $row}",
    ] {
        let yaml = format!(
            r#"
verification: body forms
inputs:
  rows: {{type: array, default: [1, 2, 3]}}
flows:
  observe:
    params: {{row: {{type: integer}}}}
    steps:
      - uses: db/observe
        with: {{connection: 'sqlite::memory:', sql: 'select 1 as n where ? > 0', params: [$params.row], timeout: 20ms, until: {{row_count: 1}}}}
criteria:
  - id: AC-1
    description: Each row is observed
    checks:
      - id: AC-1.1
        steps:
          - for_each: $inputs.rows
            max: 3
            as: row
            {body}
"#
        );
        let (state, trace) = run(&yaml).await;
        assert_eq!(state, VerdictState::Pass);
        let iterations: Vec<_> = trace
            .events()
            .iter()
            .filter_map(|e| match e.payload {
                EventPayload::AssertionEvaluated { iteration, .. } => iteration,
                _ => None,
            })
            .collect();
        assert_eq!(iterations, vec![0, 1, 2]);
        let (state, trace) = run(&yaml.replace("default: [1, 2, 3]", "default: [1, 0, 3]")).await;
        assert_eq!(state, VerdictState::Fail);
        let states: Vec<_> = trace
            .events()
            .iter()
            .filter_map(|e| match e.payload {
                EventPayload::AssertionEvaluated {
                    iteration, state, ..
                } => Some((iteration, state)),
                _ => None,
            })
            .collect();
        assert_eq!(
            states,
            vec![
                (Some(0), VerdictState::Pass),
                (Some(1), VerdictState::Fail),
                (Some(2), VerdictState::Pass)
            ]
        );
    }
}

#[tokio::test]
async fn loop_reads_the_observed_list_once_and_validates_every_row() {
    let yaml = r#"
verification: observed rows
criteria:
  - id: AC-1
    description: Every observed row is positive
    checks:
      - id: AC-1.1
        steps:
          - id: list
            uses: db/query
            with: {connection: 'sqlite::memory:', sql: 'select 1 as n union all select 0 union all select 3'}
          - for_each: $steps.list.outputs.rows
            max: 3
            as: row
            steps:
              - id: inspect
                uses: db/query
                with: {connection: 'sqlite::memory:', sql: 'select ? as n', params: [$row.n]}
        assertions: ['$steps.inspect.outputs.rows[0].n > 0']
"#;
    let (state, trace) = run(yaml).await;
    assert_eq!(state, VerdictState::Fail);
    assert_eq!(
        trace
            .events()
            .iter()
            .filter(|e| matches!(e.payload, EventPayload::StepStarted { .. }))
            .count(),
        4
    );
    assert_eq!(
        trace
            .events()
            .iter()
            .filter(|e| matches!(
                e.payload,
                EventPayload::AssertionEvaluated {
                    iteration: Some(_),
                    ..
                }
            ))
            .count(),
        3
    );
}

#[tokio::test]
async fn an_empty_obligation_is_not_hidden_by_an_ordinary_passing_judgment() {
    let yaml = r#"
verification: empty rows
inputs: {rows: {type: array, default: []}}
criteria:
  - id: AC-1
    description: Every row is observed
    checks:
      - id: AC-1.1
        steps:
          - uses: db/observe
            with: {connection: 'sqlite::memory:', sql: 'select 1', until: {row_count: 1}}
          - for_each: $inputs.rows
            max: 3
            uses: db/observe
            with: {connection: 'sqlite::memory:', sql: 'select 1', until: {row_count: 1}}
"#;
    assert_eq!(
        run(yaml).await.0,
        VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation)
    );
}
