//! Catalog-aware validation kept outside the already-budgeted structural
//! validator. The action implementation crate is injected by callers.

use crate::source::SourcePathSegment;
use crate::{Step, ValidationError, VerificationDefinition};

pub(crate) fn unknown_actions(
    definition: &VerificationDefinition,
    action_for: &dyn Fn(&str) -> Option<Vec<String>>,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    check_steps(
        definition,
        &definition.setup,
        "setup",
        &[],
        action_for,
        &mut errors,
    );
    check_steps(
        definition,
        &definition.teardown,
        "teardown",
        &[],
        action_for,
        &mut errors,
    );
    for (criterion_index, criterion) in definition.criteria.iter().enumerate() {
        for (check_index, check) in criterion.checks.iter().enumerate() {
            check_steps(
                definition,
                &check.steps,
                "steps",
                &[criterion_index, check_index],
                action_for,
                &mut errors,
            );
        }
    }
    errors
}

fn check_steps(
    definition: &VerificationDefinition,
    steps: &[Step],
    collection: &str,
    scope: &[usize],
    action_for: &dyn Fn(&str) -> Option<Vec<String>>,
    errors: &mut Vec<ValidationError>,
) {
    for (step_index, step) in steps.iter().enumerate() {
        let Some(uses) = step.uses.as_deref() else {
            continue;
        };
        if action_for(uses).is_some() {
            continue;
        }
        let mut path = if scope.is_empty() {
            vec![SourcePathSegment::key(collection)]
        } else {
            vec![
                SourcePathSegment::key("criteria"),
                SourcePathSegment::index(scope[0]),
                SourcePathSegment::key("checks"),
                SourcePathSegment::index(scope[1]),
                SourcePathSegment::key(collection),
            ]
        };
        path.extend([
            SourcePathSegment::index(step_index),
            SourcePathSegment::key("uses"),
        ]);
        errors.push(ValidationError::UnknownAction {
            uses: uses.to_string(),
            location: definition.source_map.scalar_location(&path, uses),
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::{VerificationDefinition, validate, validate_with_action_catalog};

    const UNKNOWN: &str = r#"
verification: unknown
criteria:
  - id: AC-1
    description: unknown action
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/wait-nonexistent
"#;

    #[test]
    fn injected_catalog_rejects_unknown_but_bare_validation_does_not() {
        let definition = VerificationDefinition::from_yaml_str(UNKNOWN).unwrap();
        validate(&definition).expect("schema-only validation has no catalog");
        let errors = validate_with_action_catalog(&definition, &|_| None).unwrap_err();
        let unknown = errors
            .iter()
            .find(|error| error.to_string().contains("ui/wait-nonexistent"))
            .expect("unknown action error");
        assert_eq!(unknown.location().unwrap().line, 9);
    }
}
