//! Reject statically known mutations of enclosing process owners (#579).
use std::collections::HashSet;

use crate::{Step, ValidationError, VerificationDefinition};

fn starts(steps: &[Step]) -> HashSet<&str> {
    steps
        .iter()
        .flat_map(Step::actions)
        .filter(|step| step.uses.as_deref() == Some("process/start"))
        .filter_map(|step| step.id.as_deref())
        .collect()
}

fn check(
    steps: &[Step],
    site: &str,
    outer: &HashSet<&str>,
    mutable_scope: bool,
    errs: &mut Vec<ValidationError>,
) {
    for step in steps.iter().flat_map(Step::actions) {
        if !matches!(
            step.uses.as_deref(),
            Some("process/stop" | "process/restart")
        ) {
            continue;
        }
        let known_outer = step
            .with
            .as_mapping()
            .and_then(|map| map.get(serde_yml::Value::String("handle".into())))
            .and_then(serde_yml::Value::as_str)
            .and_then(|raw| raw.strip_prefix("$setup."))
            .and_then(|raw| raw.split_once(".outputs."))
            .is_some_and(|(id, _)| outer.contains(id));
        if !mutable_scope || known_outer {
            errs.push(ValidationError::ProcessMutationScope {
                site: site.into(),
                action: step.uses_name().into(),
            });
        }
    }
}

pub(crate) fn validate(definition: &VerificationDefinition, errs: &mut Vec<ValidationError>) {
    let leaf = starts(&definition.setup);
    check(&definition.setup, "leaf setup", &leaf, false, errs);
    check(&definition.teardown, "leaf teardown", &leaf, false, errs);
    for (name, fixture) in &definition.fixtures {
        check(
            &fixture.up,
            &format!("fixture `{name}` up"),
            &leaf,
            true,
            errs,
        );
        check(
            &fixture.down,
            &format!("fixture `{name}` down"),
            &leaf,
            true,
            errs,
        );
    }
    for criterion in &definition.criteria {
        let mut outer = leaf.clone();
        outer.extend(starts(&criterion.setup));
        check(
            &criterion.setup,
            &format!("criterion `{}` setup", criterion.id),
            &outer,
            false,
            errs,
        );
        check(
            &criterion.teardown,
            &format!("criterion `{}` teardown", criterion.id),
            &outer,
            false,
            errs,
        );
        for ch in &criterion.checks {
            let site = format!("check `{}`", ch.id);
            for steps in [&ch.setup, &ch.steps, &ch.teardown] {
                check(steps, &site, &outer, true, errs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enclosing_handles_cannot_be_mutated_even_through_output_aliases() {
        for action in ["process/stop", "process/restart"] {
            let yaml = format!(
                r#"
verification: Ownership
setup:
  - id: shared
    uses: process/start
    with: {{ command: sleep, args: ["30"] }}
    outputs: {{ resource: handle }}
criteria:
  - id: AC-1
    description: Shared processes are read-only to checks.
    checks:
      - id: AC-1.1
        steps:
          - uses: {action}
            with: {{ handle: $setup.shared.outputs.resource }}
        assertions: ["true"]
"#
            );
            let definition = VerificationDefinition::from_yaml_str(&yaml).unwrap();
            let errors =
                crate::validate_with_contract_outputs(&definition, &|_| vec!["handle".into()])
                    .unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|error| matches!(error, ValidationError::ProcessMutationScope { .. }))
            );
            let valid =
                VerificationDefinition::from_yaml_str(&yaml.replace(action, "process/inspect"))
                    .unwrap();
            let mut errors = Vec::new();
            validate(&valid, &mut errors);
            assert!(errors.is_empty());
        }
    }

    #[test]
    fn check_setup_process_can_be_restarted_but_criterion_process_cannot() {
        let yaml = r#"
verification: Ownership
criteria:
  - id: AC-1
    description: Mutation belongs to a check.
    checks:
      - id: AC-1.1
        setup:
          - id: local
            uses: process/start
            with: { command: sleep, args: ["30"] }
        steps:
          - uses: process/restart
            with: { handle: $setup.local.outputs.handle }
        assertions: ["true"]
"#;
        let definition = VerificationDefinition::from_yaml_str(yaml).unwrap();
        let mut errors = Vec::new();
        validate(&definition, &mut errors);
        assert!(errors.is_empty());
        let mut definition = definition;
        definition.criteria[0].setup = std::mem::take(&mut definition.criteria[0].checks[0].setup);
        validate(&definition, &mut errors);
        assert_eq!(errors.len(), 1);
    }
}
