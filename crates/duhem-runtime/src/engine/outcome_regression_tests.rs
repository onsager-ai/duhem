use std::collections::{BTreeMap, BTreeSet};

use duhem_actions::Outcome;
use duhem_judge::{InconclusiveCause, VerdictState};

use super::outcome::{StepEvidence, implicit_judgment_outcomes};

fn check(steps: &str) -> duhem_schema::Check {
    serde_yml::from_str(&format!("id: AC-1.1\nsteps:\n{steps}")).unwrap()
}

fn evidence(outcome: Outcome, with: &str) -> StepEvidence {
    StepEvidence {
        with: serde_yml::from_str(with).unwrap(),
        outputs: BTreeMap::new(),
        skip_reason: None,
        catalog_reference: None,
        outcome: Some(outcome),
        detail: None,
    }
}

#[test]
fn timed_out_actuator_between_judgments_poison_sequence_and_names_step() {
    let check = check(
        "  - uses: ui/assert-element\n  - id: send_message\n    uses: ui/click\n    with: { timeout: 10s }\n  - uses: ui/assert-element\n",
    );
    let evidence = vec![
        StepEvidence {
            outputs: BTreeMap::from([("satisfied".into(), serde_json::json!(true))]),
            ..evidence(Outcome::Ok, "{}")
        },
        evidence(Outcome::Timeout, "{ timeout: 10s }"),
        StepEvidence::skipped("gated after `send_message`".into()),
    ];
    let outcomes = implicit_judgment_outcomes(
        &check,
        |uses| uses == "ui/assert-element",
        |_| true,
        &evidence,
        false,
        false,
        &BTreeSet::new(),
    );
    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        outcomes[1].state,
        VerdictState::Inconclusive(InconclusiveCause::Timeout)
    );
    assert_eq!(
        outcomes[1].detail.as_deref(),
        Some("step `send_message` timed out after 10s")
    );
}

#[test]
fn unknown_action_without_assertions_names_label_and_uses() {
    let check = check("  - id: wait_two_seconds\n    uses: ui/wait-nonexistent\n");
    let outcomes = implicit_judgment_outcomes(
        &check,
        |_| false,
        |_| false,
        &[evidence(Outcome::Error, "{}")],
        false,
        false,
        &BTreeSet::new(),
    );
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].state,
        VerdictState::Inconclusive(InconclusiveCause::MissingObservation)
    );
    let detail = outcomes[0].detail.as_deref().unwrap();
    assert!(detail.contains("wait_two_seconds"), "{detail}");
    assert!(detail.contains("ui/wait-nonexistent"), "{detail}");
}

#[test]
fn environment_failure_contributes_for_each_non_gated_actuator() {
    let check = check(
        "  - id: first\n    uses: ui/click\n  - id: gated\n    uses: ui/type\n  - id: second\n    uses: ui/select\n",
    );
    let evidence = vec![
        evidence(Outcome::Error, "{}"),
        StepEvidence::skipped("if condition did not match".into()),
        evidence(Outcome::Error, "{}"),
    ];
    let outcomes = implicit_judgment_outcomes(
        &check,
        |_| false,
        |_| true,
        &evidence,
        true,
        false,
        &BTreeSet::new(),
    );
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|outcome| {
        outcome.state == VerdictState::Inconclusive(InconclusiveCause::EnvironmentError)
    }));
}

#[test]
fn gating_cleanup_and_manual_control_obey_execution_failure_rules() {
    let check = check(
        "  - id: gated\n    uses: ui/click\n  - id: cleanup\n    uses: ui/click\n  - id: manual\n    uses: ui/assert-url\n    outputs: { satisfied: satisfied }\n",
    );
    let evidence = vec![
        StepEvidence::skipped("if condition did not match".into()),
        evidence(Outcome::Error, "{}"),
        evidence(Outcome::Timeout, "{ timeout: 2s }"),
    ];
    let outcomes = implicit_judgment_outcomes(
        &check,
        |uses| uses == "ui/assert-url",
        |_| true,
        &evidence,
        false,
        false,
        &BTreeSet::from([1]),
    );
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].label, "manual");
    assert_eq!(
        outcomes[0].state,
        VerdictState::Inconclusive(InconclusiveCause::Timeout)
    );
}
