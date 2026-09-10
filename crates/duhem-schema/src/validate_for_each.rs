//! Validation for `for_each:` (spec #443 Tier 1).
//!
//! Bounded iteration is permitted only in non-judging contexts —
//! `setup:`, `teardown:`, fixture `up:`/`down:`, and the criterion-/
//! check-level equivalents (§10.3.6) — never inside a check's
//! `steps:` (Tier 2, gated behind #509: a shrunken claim set must be
//! visible in the verdict before a loop can wrap judging steps).
//!
//! Split out of `validate_lifecycle.rs` because `for_each` has its own
//! rule set (mandatory `max:`, a depth-1 body cap, and the `as:`
//! binding's scope) distinct from `if:`'s value-condition rules, even
//! though both share the same lifecycle surface and the same
//! `validate_nested_lifecycle_block_in_loop` walk for a step's own
//! `with:`/`if:` and id bookkeeping.

use std::collections::HashMap;

use crate::source::SourcePathSegment;
use crate::step::Step;
use crate::validate_error::ValidationError;
use crate::validate_lifecycle::validate_nested_lifecycle_block_in_loop;
use crate::verification::VerificationDefinition;

/// Validate every `for_each:` in the definition: Tier 2 rejection
/// inside a check's `steps:`, and the full Tier 1 rule set (`max:`
/// required, wrapper fields, depth cap, `as:` scope) everywhere else.
pub(crate) fn validate_for_each(
    v: &VerificationDefinition,
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    errs: &mut Vec<ValidationError>,
) {
    for criterion in &v.criteria {
        for check in &criterion.checks {
            for (idx, step) in check.steps.iter().enumerate() {
                if step.for_each.is_none() {
                    continue;
                }
                let expr = step.for_each.as_ref().expect("checked above");
                let location = check_step_for_each_location(v, criterion, check, idx, &expr.raw);
                errs.push(ValidationError::ForEachUnavailableInCheck {
                    criterion: criterion.id.clone(),
                    check: check.id.clone(),
                    location,
                });
            }
        }
    }

    validate_for_each_list(
        v,
        outputs_for,
        &v.setup,
        "setup",
        "setup",
        &[SourcePathSegment::key("setup")],
        errs,
    );
    validate_for_each_list(
        v,
        outputs_for,
        &v.teardown,
        "teardown",
        "teardown",
        &[SourcePathSegment::key("teardown")],
        errs,
    );
    for (fixture, lifecycle) in &v.fixtures {
        validate_for_each_list(
            v,
            outputs_for,
            &lifecycle.up,
            "up",
            &format!("fixture `{fixture}`"),
            &[
                SourcePathSegment::key("fixtures"),
                SourcePathSegment::key(fixture.as_str()),
                SourcePathSegment::key("up"),
            ],
            errs,
        );
        validate_for_each_list(
            v,
            outputs_for,
            &lifecycle.down,
            "down",
            &format!("fixture `{fixture}`"),
            &[
                SourcePathSegment::key("fixtures"),
                SourcePathSegment::key(fixture.as_str()),
                SourcePathSegment::key("down"),
            ],
            errs,
        );
    }
    for (criterion_index, criterion) in v.criteria.iter().enumerate() {
        let criterion_label = format!("criterion `{}`", criterion.id);
        validate_for_each_list(
            v,
            outputs_for,
            &criterion.setup,
            "setup",
            &criterion_label,
            &[
                SourcePathSegment::key("criteria"),
                SourcePathSegment::index(criterion_index),
                SourcePathSegment::key("setup"),
            ],
            errs,
        );
        validate_for_each_list(
            v,
            outputs_for,
            &criterion.teardown,
            "teardown",
            &criterion_label,
            &[
                SourcePathSegment::key("criteria"),
                SourcePathSegment::index(criterion_index),
                SourcePathSegment::key("teardown"),
            ],
            errs,
        );
        for (check_index, check) in criterion.checks.iter().enumerate() {
            let check_label = format!("criterion `{}` / check `{}`", criterion.id, check.id);
            validate_for_each_list(
                v,
                outputs_for,
                &check.setup,
                "setup",
                &check_label,
                &[
                    SourcePathSegment::key("criteria"),
                    SourcePathSegment::index(criterion_index),
                    SourcePathSegment::key("checks"),
                    SourcePathSegment::index(check_index),
                    SourcePathSegment::key("setup"),
                ],
                errs,
            );
            validate_for_each_list(
                v,
                outputs_for,
                &check.teardown,
                "teardown",
                &check_label,
                &[
                    SourcePathSegment::key("criteria"),
                    SourcePathSegment::index(criterion_index),
                    SourcePathSegment::key("checks"),
                    SourcePathSegment::index(check_index),
                    SourcePathSegment::key("teardown"),
                ],
                errs,
            );
        }
    }
}

/// Location of a check step's `for_each:` scalar, for the Tier 2
/// rejection. Mirrors `SourceMap::check_step_with_location`'s
/// positional addressing (criteria/checks are indexed, not keyed).
fn check_step_for_each_location(
    v: &VerificationDefinition,
    criterion: &crate::criterion::Criterion,
    check: &crate::criterion::Check,
    step_index: usize,
    raw: &str,
) -> Option<crate::SourceLocation> {
    let criterion_index = v.criteria.iter().position(|c| c.id == criterion.id)?;
    let check_index = criterion
        .checks
        .iter()
        .position(|c| c.id == check.id)
        .unwrap_or(0);
    let path = [
        SourcePathSegment::key("criteria"),
        SourcePathSegment::index(criterion_index),
        SourcePathSegment::key("checks"),
        SourcePathSegment::index(check_index),
        SourcePathSegment::key("steps"),
        SourcePathSegment::index(step_index),
        SourcePathSegment::key("for_each"),
    ];
    v.source_map.scalar_location(&path, raw)
}

/// Validate every `for_each:` step directly inside one lifecycle step
/// list. `phase` is `"setup"`/`"teardown"`/`"up"`/`"down"` for message
/// text; `label` is the enclosing scope's human prefix (e.g. `` leaf ``
/// or `` criterion `AC-1` ``).
fn validate_for_each_list(
    v: &VerificationDefinition,
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    steps: &[Step],
    phase: &str,
    label: &str,
    source_path_prefix: &[SourcePathSegment],
    errs: &mut Vec<ValidationError>,
) {
    for (idx, step) in steps.iter().enumerate() {
        if step.for_each.is_none() {
            continue;
        }
        let expr = step.for_each.as_ref().expect("checked above");
        let mut step_path = source_path_prefix.to_vec();
        step_path.push(SourcePathSegment::index(idx));
        let mut for_each_path = step_path.clone();
        for_each_path.push(SourcePathSegment::key("for_each"));
        let location = v.source_map.scalar_location(&for_each_path, &expr.raw);
        let site = format!("{label} {phase} step {idx}");

        if step.max.is_none() {
            errs.push(ValidationError::ForEachMaxRequired {
                site: site.clone(),
                location,
            });
        }
        for (field, present) in [
            ("id", step.id.is_some()),
            ("outputs", !step.outputs.is_empty()),
            ("secret_outputs", !step.secret_outputs.is_empty()),
        ] {
            if present {
                errs.push(ValidationError::ForEachWrapperFieldNotAllowed {
                    site: site.clone(),
                    field,
                    location,
                });
            }
        }

        let active_loop = step.as_binding.as_deref();
        let Some(body) = &step.steps else {
            // `uses:`/`call:` single-action body forms: nothing to
            // recurse into. Their own `with:` is already validated as
            // part of this same step by the caller's ordinary
            // `if:`/`with:` walk (leaf setup/teardown's inline
            // closures, or `validate_nested_lifecycle_block_in_loop`
            // for the criterion-/check-level and for_each-body cases),
            // which resolves a `for_each` wrapper's `$<as>` references
            // against its own `as:` binding. A `call:` body form's
            // flow-param arity/type checking happens in
            // `flows::validate_lifecycle_dispatch`.
            continue;
        };
        for (inner_index, inner) in body.iter().enumerate() {
            let mut inner_path = step_path.clone();
            inner_path.push(SourcePathSegment::key("steps"));
            inner_path.push(SourcePathSegment::index(inner_index));
            if let Some(inner_for_each) = &inner.for_each {
                let mut inner_for_each_path = inner_path.clone();
                inner_for_each_path.push(SourcePathSegment::key("for_each"));
                errs.push(ValidationError::ForEachDepthExceeded {
                    site: format!("{site} body step {inner_index}"),
                    field: "for_each",
                    location: v
                        .source_map
                        .scalar_location(&inner_for_each_path, &inner_for_each.raw),
                });
            }
            if inner.steps.is_some() {
                let mut inner_steps_path = inner_path.clone();
                inner_steps_path.push(SourcePathSegment::key("steps"));
                errs.push(ValidationError::ForEachDepthExceeded {
                    site: format!("{site} body step {inner_index}"),
                    field: "steps",
                    location: v.source_map.node_location(&inner_steps_path),
                });
            }
        }

        let mut body_path_prefix = step_path.clone();
        body_path_prefix.push(SourcePathSegment::key("steps"));
        validate_nested_lifecycle_block_in_loop(
            v,
            outputs_for,
            body,
            "for_each body",
            &body_path_prefix,
            &site,
            &HashMap::new(),
            active_loop,
            errs,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::VerificationDefinition;

    fn parse(y: &str) -> VerificationDefinition {
        VerificationDefinition::from_yaml_str(y).expect("parse")
    }

    fn errs(y: &str) -> Vec<ValidationError> {
        crate::validate(&parse(y)).unwrap_err()
    }

    #[test]
    fn all_three_body_forms_validate() {
        let uses_form = "verification: x\ninputs:\n  rows: { type: array }\nsetup:\n  - for_each: $inputs.rows\n    max: 5\n    as: row\n    uses: cli/invoke\n    with: { command: [echo, $row] }\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        crate::validate(&parse(uses_form)).expect("uses: body form validates");

        let call_form = "verification: x\ninputs:\n  rows: { type: array }\nflows:\n  delete_row:\n    steps:\n      - uses: cli/invoke\nsetup:\n  - for_each: $inputs.rows\n    max: 5\n    as: row\n    call: delete_row\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        crate::validate(&parse(call_form)).expect("call: body form validates");

        let steps_form = "verification: x\ninputs:\n  rows: { type: array }\nsetup:\n  - for_each: $inputs.rows\n    max: 5\n    as: row\n    steps:\n      - uses: cli/invoke\n        with: { command: [echo, $row] }\n      - uses: cli/invoke\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        crate::validate(&parse(steps_form)).expect("steps: body form validates");
    }

    #[test]
    fn declaring_zero_or_two_body_forms_is_rejected() {
        let neither = "verification: x\nsetup:\n  - for_each: $inputs.rows\n    max: 5\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        let e = errs(neither);
        assert!(
            e.iter().any(|e| e
                .to_string()
                .contains("exactly one of `uses:`, `call:`, or `steps:`")),
            "{e:?}"
        );

        let both = "verification: x\nsetup:\n  - for_each: $inputs.rows\n    max: 5\n    uses: cli/invoke\n    steps: [{ uses: cli/invoke }]\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        let e = errs(both);
        assert!(
            e.iter()
                .any(|e| e.to_string().contains("not more than one")),
            "{e:?}"
        );
    }

    #[test]
    fn max_is_required_with_a_location() {
        let y = "verification: x\nsetup:\n  - for_each: $inputs.rows\n    uses: cli/invoke\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        let e = errs(y);
        assert!(
            e.iter().any(|e| matches!(
                e,
                ValidationError::ForEachMaxRequired {
                    location: Some(_),
                    ..
                }
            )),
            "{e:?}"
        );
    }

    #[test]
    fn for_each_inside_a_check_is_tier_2_and_names_509_with_a_location() {
        let y = "verification: x\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        steps:\n          - for_each: $inputs.rows\n            max: 5\n            uses: cli/invoke\n        assertions: [\"true\"]\n";
        let e = errs(y);
        assert!(
            e.iter().any(|e| matches!(
                e,
                ValidationError::ForEachUnavailableInCheck {
                    location: Some(_),
                    ..
                }
            ) && e.to_string().contains("#509")),
            "{e:?}"
        );
    }

    #[test]
    fn depth_greater_than_one_is_rejected() {
        let nested_for_each = "verification: x\nsetup:\n  - for_each: $inputs.rows\n    max: 5\n    as: row\n    steps:\n      - for_each: $inputs.cols\n        max: 5\n        uses: cli/invoke\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        let e = errs(nested_for_each);
        assert!(
            e.iter().any(|e| matches!(
                e,
                ValidationError::ForEachDepthExceeded {
                    field: "for_each",
                    location: Some(_),
                    ..
                }
            )),
            "{e:?}"
        );

        let nested_steps = "verification: x\nsetup:\n  - for_each: $inputs.rows\n    max: 5\n    as: row\n    steps:\n      - steps: [{ uses: cli/invoke }]\n        uses: cli/invoke\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        let e = errs(nested_steps);
        assert!(
            e.iter().any(|e| matches!(
                e,
                ValidationError::ForEachDepthExceeded {
                    field: "steps",
                    location: Some(_),
                    ..
                }
            )),
            "{e:?}"
        );
    }

    #[test]
    fn as_binding_is_in_scope_inside_the_body_and_out_of_scope_outside() {
        let in_scope = "verification: x\ninputs:\n  rows: { type: array }\nsetup:\n  - for_each: $inputs.rows\n    max: 5\n    as: row\n    uses: cli/invoke\n    with: { command: [echo, $row] }\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        crate::validate(&parse(in_scope)).expect("$row resolves inside its own for_each body");

        let out_of_scope = "verification: x\nsetup:\n  - for_each: $inputs.rows\n    max: 5\n    as: row\n    uses: cli/invoke\n    with: { command: [echo, $row] }\n  - uses: cli/invoke\n    with: { command: [echo, $row] }\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        let e = errs(out_of_scope);
        assert!(
            e.iter().any(|e| matches!(
                e,
                ValidationError::LoopVariableOutOfScope {
                    location: Some(_),
                    ..
                }
            )),
            "{e:?}"
        );
    }

    #[test]
    fn wrapper_may_not_declare_id_outputs_or_secret_outputs() {
        let y = "verification: x\nsetup:\n  - id: loop\n    for_each: $inputs.rows\n    max: 5\n    uses: cli/invoke\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        let e = errs(y);
        assert!(
            e.iter().any(|e| matches!(
                e,
                ValidationError::ForEachWrapperFieldNotAllowed { field: "id", .. }
            )),
            "{e:?}"
        );
    }

    /// #440 R3 parity: a `for_each:` source expression gets exactly
    /// the same static reference resolution `if:` already gets — a
    /// typo'd `$setup.<step>.outputs.<name>` is a validate-time error
    /// with a source location, not a silent pass that only fails at
    /// runtime. Pins both halves: the bad reference is rejected, and
    /// the corrected one still validates.
    #[test]
    fn for_each_source_is_statically_validated_like_if_condition() {
        let vd = |output: &str| {
            format!(
                "verification: x\nsetup:\n  - id: rows\n    uses: cli/invoke\n    with: {{ command: [echo] }}\n  - for_each: $setup.rows.outputs.{output}\n    max: 5\n    uses: cli/invoke\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n"
            )
        };
        let contract_outputs = |uses: &str| -> Vec<String> {
            if uses == "cli/invoke" {
                vec!["stdout".into()]
            } else {
                Vec::new()
            }
        };

        let bad = parse(&vd("stdoutt"));
        let e = crate::validate_with_contract_outputs(&bad, &contract_outputs).unwrap_err();
        assert!(
            e.iter().any(|e| matches!(
                e,
                ValidationError::InvalidStepCondition {
                    location: Some(_),
                    ..
                }
            ) && e.to_string().contains("undeclared output `stdoutt`")),
            "{e:?}"
        );

        let good = parse(&vd("stdout"));
        crate::validate_with_contract_outputs(&good, &contract_outputs)
            .expect("a correctly-spelled for_each source validates");
    }
}
