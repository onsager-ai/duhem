//! Step groups (#614) through real SQLite actions: entry-time `if:`, shared
//! failure-gate state, skipped-with-cause records and Tier 2 claim counting.
use std::{collections::BTreeMap, sync::Arc};

use duhem_evidence::{EventPayload, SqliteStore, StepOutcome, Trace};
use duhem_judge::VerdictState;
use duhem_runtime::Engine;

const FAIL: &str =
    "uses: db/query, with: { connection: 'sqlite::memory:', sql: 'select * from missing' }";
const OK: &str = "uses: db/query, with: { connection: 'sqlite::memory:', sql: 'select 1 as v' }";
const OK_ALWAYS: &str =
    "if: always, uses: db/query, with: { connection: 'sqlite::memory:', sql: 'select 1 as v' }";
const EMPTY_PROBE: &str = "id: probe, uses: db/query, with: { connection: 'sqlite::memory:', sql: 'select 1 as v where 0' }";
const JUDGE: &str = "uses: db/observe, with: { connection: 'sqlite::memory:', sql: 'select 1', until: { row_count: 1 } }";

struct Ran {
    outcome: duhem_runtime::RunOutcome,
    trace: Trace,
}

impl Ran {
    /// `(step_index, "ok" | "error" | "skipped: <reason>")`, in step order.
    fn steps(&self) -> Vec<(u32, String)> {
        self.trace
            .events()
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::StepFinished {
                    step_index,
                    outcome,
                    ..
                } => Some((
                    *step_index,
                    match outcome {
                        StepOutcome::Ok => "ok".to_string(),
                        StepOutcome::Skipped { reason, .. } => format!("skipped: {reason}"),
                        _ => "error".to_string(),
                    },
                )),
                _ => None,
            })
            .collect()
    }
}

async fn run(steps: &str) -> Ran {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("verification.yml");
    std::fs::write(
        &path,
        format!(
            "verification: groups\ncriteria:\n  - id: AC-1\n    description: groups\n    checks:\n      - id: AC-1.1\n        steps:\n{steps}        assertions: [\"true\"]\n"
        ),
    )
    .unwrap();
    let duhem_schema::Loaded::Leaf {
        definition: def, ..
    } = duhem_schema::load(&path).unwrap()
    else {
        panic!("expected leaf")
    };
    let store = Arc::new(
        SqliteStore::open(dir.path().join("evidence.db"))
            .await
            .unwrap(),
    );
    let outcome = Engine::new()
        .with_store(store.clone())
        .run_with_metadata(&def, BTreeMap::new())
        .await
        .unwrap();
    let trace = Trace::from_store(store.as_ref(), &outcome.run_id)
        .await
        .unwrap();
    Ran { outcome, trace }
}

fn group(label: &str, condition: &str, inner: &[&str]) -> String {
    let steps: String = inner
        .iter()
        .map(|step| format!("              - {{ {step} }}\n"))
        .collect();
    format!("          - id: {label}\n            if: {condition}\n            steps:\n{steps}")
}

fn step(body: &str) -> String {
    format!("          - {{ {body} }}\n")
}

#[tokio::test]
async fn group_conditions_are_evaluated_once_at_entry_against_the_failure_state() {
    let steps = [
        step(FAIL),
        group("keep", "always", &[OK]),
        group("diag", "failure", &[OK]),
        group("happy", "success", &[OK, OK]),
    ]
    .concat();
    let ran = run(&steps).await;
    let seen = ran.steps();
    assert_eq!(seen[0].1, "error");
    assert_eq!(seen[1].1, "ok");
    assert_eq!(seen[2].1, "ok");
    for (index, detail) in &seen[3..] {
        assert!(
            detail.starts_with("skipped: group `happy`"),
            "step {index} should name its group: {detail}"
        );
    }
    assert_eq!(seen.len(), 5);
    let groups: Vec<_> = ran
        .trace
        .events()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::StepStarted { group, .. } => group.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(groups[0], ["keep"]);
    assert_eq!(groups[3], ["happy"]);
}

#[tokio::test]
async fn a_failure_inside_a_group_gates_later_steps_inside_and_after_it() {
    let steps = [
        group("work", "success", &[OK, FAIL, OK, OK_ALWAYS]),
        step(OK),
        group("diag", "failure", &[OK]),
    ]
    .concat();
    let ran = run(&steps).await;
    let seen: Vec<_> = ran.steps().into_iter().map(|(_, detail)| detail).collect();
    assert_eq!(seen[0], "ok");
    assert_eq!(seen[1], "error");
    assert!(
        seen[2].starts_with("skipped: blocked by failed step"),
        "{seen:?}"
    );
    assert_eq!(seen[3], "ok", "`if: always` inside a group still runs");
    assert!(
        seen[4].starts_with("skipped: blocked by failed step"),
        "{seen:?}"
    );
    assert_eq!(
        seen[5], "ok",
        "`if: failure` group runs after an inner failure"
    );
}

#[tokio::test]
async fn a_value_gated_group_counts_its_judging_steps_as_gated() {
    let steps = [
        step(EMPTY_PROBE),
        group(
            "claims",
            "$steps.probe.outputs.row_count > 0",
            &[JUDGE, JUDGE],
        ),
    ]
    .concat();
    let ran = run(&steps).await;
    assert_eq!(ran.outcome.verdict.state, VerdictState::Pass);
    assert_eq!(ran.outcome.gated_checks.values().sum::<u32>(), 2);
    assert!(ran.steps()[1].1.starts_with("skipped: group `claims`"));
}
