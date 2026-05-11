//! End-to-end round-trip and validation against the on-disk fixtures.
//!
//! `create-workspace.yml` is the spec's worked example; it must
//! parse, validate, and re-serialize byte-equivalent to the source
//! (modulo trailing newline). Negative fixtures live alongside it and
//! each pins a single `ValidationError` variant.

use duhem_schema::{ValidationError, VerificationDefinition, validate};

const POSITIVE: &str = include_str!("../fixtures/create-workspace.yml");

#[test]
fn worked_example_parses_and_validates() {
    let v = VerificationDefinition::from_yaml_str(POSITIVE).expect("parse");
    validate(&v).expect("validate");
}

#[test]
fn worked_example_round_trips_byte_equivalent() {
    let v = VerificationDefinition::from_yaml_str(POSITIVE).expect("parse");
    let out = v.to_yaml_string().expect("serialize");
    assert_eq!(
        normalize(&out),
        normalize(POSITIVE),
        "fixture is not round-trip stable; re-canonicalize via\n    cargo run -p duhem-schema --example canonicalize -- crates/duhem-schema/fixtures/create-workspace.yml",
    );
}

fn normalize(s: &str) -> &str {
    s.trim_end_matches('\n')
}

#[test]
fn malformed_yaml_carries_location() {
    let bad = "verification: x\ncriteria:\n\t- id: AC-1\n";
    let err = VerificationDefinition::from_yaml_str(bad).unwrap_err();
    assert!(err.location().is_some(), "expected line/col info: {err}");
}

macro_rules! negative_fixture {
    ($name:ident, $file:literal, $variant:pat) => {
        #[test]
        fn $name() {
            let src = include_str!(concat!("../fixtures/negative/", $file));
            let v = VerificationDefinition::from_yaml_str(src).expect("fixture must parse");
            let errs = validate(&v).expect_err("fixture must fail validation");
            assert!(
                errs.iter().any(|e| matches!(e, $variant)),
                "expected variant; got errors: {errs:#?}",
            );
        }
    };
}

negative_fixture!(
    negative_duplicate_criterion_id,
    "duplicate_criterion_id.yml",
    ValidationError::DuplicateCriterionId { .. }
);

negative_fixture!(
    negative_duplicate_check_id,
    "duplicate_check_id.yml",
    ValidationError::DuplicateCheckId { .. }
);

negative_fixture!(
    negative_duplicate_step_id,
    "duplicate_step_id.yml",
    ValidationError::DuplicateStepId { .. }
);

negative_fixture!(
    negative_unresolved_step_ref,
    "unresolved_step_ref.yml",
    ValidationError::UnresolvedStepRef { .. }
);

negative_fixture!(
    negative_unresolved_input_ref,
    "unresolved_input_ref.yml",
    ValidationError::UnresolvedInputRef { .. }
);
