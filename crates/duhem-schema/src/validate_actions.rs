//! Catalog-aware validation kept outside the already-budgeted structural
//! validator. The action implementation crate is injected by callers.

use crate::source::SourcePathSegment;
use crate::{ValidationError, VerificationDefinition};

pub(crate) fn unknown_actions(
    definition: &VerificationDefinition,
    action_for: &dyn Fn(&str) -> Option<Vec<String>>,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for block in crate::step_walk::step_blocks(definition) {
        for (index, step) in block.steps.iter().enumerate() {
            let Some(uses) = step.uses.as_deref() else {
                continue;
            };
            if action_for(uses).is_some() {
                continue;
            }
            let mut path = block.path.clone();
            path.extend([
                SourcePathSegment::index(index),
                SourcePathSegment::key("uses"),
            ]);
            errors.push(ValidationError::UnknownAction {
                uses: uses.to_string(),
                location: definition.source_map.scalar_location(&path, uses),
            });
        }
    }
    errors
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
