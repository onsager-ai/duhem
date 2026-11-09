//! Coverage of walker-backed per-step rules, not validator semantics.
//! Separate traversals for fixture/lifecycle scopes, references and ordering,
//! conditions, loops, sessions, process checks, flows and check judgment are
//! outside this gate, as are future schema fields that are not step arrays.

use crate as duhem_schema;

include!("../tests/fixtures/coverage_helper.rs");

#[test]
fn schema_rules_visit_every_step_block() {
    use duhem_schema::{ValidationError, validate_with_action_catalog};

    for (site, id) in coverage_cases() {
        let definition = coverage_definition(
            &id,
            "unknown/coverage",
            &[
                "outputs: { capture/test: status }",
                "secret_outputs: [missing.value]",
            ],
        );
        let errors =
            validate_with_action_catalog(&definition, &|uses| (uses == "api/call").then(Vec::new))
                .unwrap_err();
        assert!(errors.iter().any(|e| matches!(e, ValidationError::UnknownAction { uses, .. } if uses == "unknown/coverage")), "unknown action at {site}: {errors:?}");
        assert!(
            errors.iter().any(
                |e| matches!(e, ValidationError::ReservedOutputPrefix { step, .. } if step == &id)
            ),
            "reserved output at {site}: {errors:?}"
        );
        assert!(
            errors.iter().any(
                |e| matches!(e, ValidationError::UndeclaredSecretOutput { step, .. } if step == &id)
            ),
            "secret output at {site}: {errors:?}"
        );
    }
}
