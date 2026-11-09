//! Step-group validation (spec #614).
//!
//! A step that declares `steps:` without `for_each:` is a group: a scope and
//! label over its children, lowered into the enclosing sequence by flow
//! expansion. These rules run on the authored tree, before lowering, because
//! the lowered sequence no longer remembers the group structure.

#[cfg(test)]
use std::collections::BTreeSet;
use std::collections::HashSet;

use crate::source::SourcePathSegment as Segment;
use crate::step_walk::step_blocks;
use crate::{PathRoot, Step, StepGroupFrame, ValidationError, VerificationDefinition};

/// A group declares `steps:` as its body and no loop.
pub(crate) fn is_group(step: &Step) -> bool {
    step.steps.is_some() && step.for_each.is_none()
}

/// Every step in the tree, parents before children, through groups and loops.
pub(crate) fn tree(steps: &[Step]) -> Vec<&Step> {
    let mut all = Vec::new();
    for step in steps {
        all.push(step);
        all.extend(tree(step.steps.as_deref().unwrap_or(&[])));
    }
    all
}

/// The leaf steps of an authored body with their group frames attached, for
/// rules that read a loop body's steps one by one.
pub(crate) fn flatten(steps: &[Step]) -> Vec<Step> {
    fn go(steps: &[Step], out: &mut Vec<Step>, key: &mut u32) {
        for (index, step) in steps.iter().enumerate() {
            if !is_group(step) {
                out.push(step.clone());
                continue;
            }
            let frame = StepGroupFrame {
                key: *key,
                label: step.id.clone().unwrap_or_else(|| index.to_string()),
                condition: step.condition.clone(),
            };
            *key += 1;
            let mut inner = Vec::new();
            go(step.steps.as_deref().unwrap_or(&[]), &mut inner, key);
            for mut leaf in inner {
                leaf.group.insert(0, frame.clone());
                out.push(leaf);
            }
        }
    }
    let mut out = Vec::new();
    go(steps, &mut out, &mut 0);
    out
}

pub(crate) fn has_groups(definition: &VerificationDefinition) -> bool {
    step_blocks(definition)
        .iter()
        .any(|block| block.steps.iter().any(is_group))
}

pub(crate) fn validate(definition: &VerificationDefinition) -> Vec<ValidationError> {
    let mut errs = Vec::new();
    for block in step_blocks(definition)
        .into_iter()
        .filter(|block| !block.nested && tree(block.steps).into_iter().any(is_group))
    {
        let mut walker = Walker {
            definition,
            site: &block.site,
            groups: tree(block.steps)
                .into_iter()
                .filter(|step| is_group(step))
                .filter_map(|step| step.id.as_deref())
                .collect(),
            errs: &mut errs,
        };
        let mut path = block.path.clone();
        walker.walk(
            block.steps,
            &mut path,
            None,
            &mut HashSet::new(),
            block.check.is_some(),
            0,
        );
        if let Some(check) = block.check {
            let prefix = &block.path[..block.path.len() - 1];
            for (index, assertion) in check.assertions.iter().enumerate() {
                let mut assertion_path = prefix.to_vec();
                assertion_path.extend([Segment::key("assertions"), Segment::index(index)]);
                assertion
                    .walk_exprs(|expr| walker.reject_group_refs(&expr.parsed, &assertion_path));
            }
        }
    }
    errs
}

struct Walker<'a> {
    definition: &'a VerificationDefinition,
    site: &'a str,
    groups: HashSet<&'a str>,
    errs: &'a mut Vec<ValidationError>,
}

impl<'a> Walker<'a> {
    fn error(&mut self, path: &[Segment], message: String) {
        self.errs.push(ValidationError::InvalidStepGroup {
            message: format!("{}: {message}", self.site),
            location: self.definition.source_map.node_location(path),
        });
    }

    fn reject_group_refs(&mut self, expr: &crate::Expr, path: &[Segment]) {
        let mut hits = Vec::new();
        expr.walk_paths(|p| {
            if p.root == PathRoot::Steps
                && let Some(id) = p.segments().first()
                && self.groups.contains(id.as_str())
            {
                hits.push(id.clone());
            }
        });
        for id in hits {
            self.error(
                path,
                format!(
                    "`$steps.{id}` names a step group, which has no outputs — reference an inner step's id instead"
                ),
            );
        }
    }

    /// `loop_ctx`: `None` outside any loop, `Some(false)` directly inside a
    /// loop body, `Some(true)` inside a loop body through a group.
    fn walk(
        &mut self,
        steps: &'a [Step],
        path: &mut Vec<Segment>,
        loop_ctx: Option<bool>,
        seen: &mut HashSet<&'a str>,
        in_check: bool,
        depth: usize,
    ) {
        for (index, step) in steps.iter().enumerate() {
            path.push(Segment::index(index));
            if let Some(id) = step.id.as_deref()
                && !seen.insert(id)
            {
                path.push(Segment::key("id"));
                self.error(
                    path,
                    format!("step {index}: duplicate step id `{id}` — ids are unique across the whole step tree, groups included"),
                );
                path.pop();
            }
            let group = is_group(step);
            if group && depth >= crate::flows::MAX_FLOW_DEPTH {
                path.push(Segment::key("steps"));
                self.error(
                    path,
                    format!(
                        "step {index}: step groups nest deeper than the limit of {}",
                        crate::flows::MAX_FLOW_DEPTH
                    ),
                );
                path.pop();
            }
            if group {
                for (field, present) in [
                    ("uses", step.uses.is_some()),
                    ("call", step.call.is_some()),
                    ("with", !step.with.is_null()),
                    ("outputs", !step.outputs.is_empty()),
                    ("secret_outputs", !step.secret_outputs.is_empty()),
                    ("as", step.as_binding.is_some()),
                    ("max", step.max.is_some()),
                ] {
                    if present {
                        path.push(Segment::key(field));
                        let message = if matches!(field, "uses" | "call") {
                            format!(
                                "step {index}: a step group declares `steps:` and no `{field}:` — every step declares exactly one of `uses:`, `call:`, or `steps:`"
                            )
                        } else {
                            format!(
                                "step {index}: `{field}:` is not allowed on a step group; only a `for_each:` step takes it"
                            )
                        };
                        self.error(path, message);
                        path.pop();
                    }
                }
            }
            if step.for_each.is_some() && loop_ctx == Some(true) {
                path.push(Segment::key("for_each"));
                self.error(
                    path,
                    format!("step {index}: `for_each:` may not appear anywhere under another `for_each:` — loop nesting is capped at depth 1"),
                );
                path.pop();
            }
            if in_check {
                self.scan_references(step, path);
            }
            if let Some(children) = &step.steps {
                let child_ctx = if group {
                    loop_ctx.map(|_| true)
                } else {
                    Some(loop_ctx.is_some())
                };
                path.push(Segment::key("steps"));
                // Lifecycle loop bodies keep their ids local to the body.
                let mut local = HashSet::new();
                let scope = if group || in_check {
                    &mut *seen
                } else {
                    &mut local
                };
                self.walk(
                    children,
                    path,
                    child_ctx,
                    scope,
                    in_check,
                    depth + usize::from(group),
                );
                path.pop();
            }
            path.pop();
        }
    }

    fn scan_references(&mut self, step: &Step, path: &mut Vec<Segment>) {
        if self.groups.is_empty() {
            return;
        }
        if let crate::StepCondition::Expr(expr) = &step.condition {
            path.push(Segment::key("if"));
            self.reject_group_refs(&expr.parsed, &path.clone());
            path.pop();
        }
        path.push(Segment::key("with"));
        let mut found = Vec::new();
        crate::source::walk_with_refs(&step.with, path, &mut |expr, _, at| {
            found.push((expr.clone(), at.to_vec()));
        });
        path.pop();
        for (expr, at) in found {
            self.reject_group_refs(&expr, &at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(steps: &str, extra: &str) -> VerificationDefinition {
        VerificationDefinition::from_yaml_str(&format!(
            "verification: groups\n{extra}criteria:\n  - id: AC-1\n    description: groups\n    checks:\n      - id: AC-1.1\n        steps:\n{steps}        assertions: [\"true\"]\n"
        ))
        .expect("parse")
    }

    fn rejected(steps: &str, extra: &str, needle: &str) {
        let errors = crate::validate(&parse(steps, extra)).expect_err("must be rejected");
        assert!(
            errors.iter().any(|error| matches!(
                error,
                ValidationError::InvalidStepGroup { message, location: Some(_) }
                    if message.contains(needle)
            )),
            "expected a located error containing `{needle}`: {errors:?}"
        );
    }

    #[test]
    fn well_formed_nested_groups_validate_and_lower_to_leaves_with_frames() {
        let mut definition = parse(
            "          - id: outer\n            steps:\n              - { id: a, uses: cli/invoke }\n              - if: failure\n                steps:\n                  - { id: b, uses: cli/invoke }\n          - { id: c, uses: cli/invoke }\n",
            "",
        );
        crate::validate(&definition).expect("groups validate");
        crate::flows::validate_and_expand(&mut definition).expect("expands");
        let steps = &definition.criteria[0].checks[0].steps;
        let shape: Vec<_> = steps
            .iter()
            .map(|step| {
                (
                    step.id.clone().unwrap(),
                    step.group
                        .iter()
                        .map(|g| g.label.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("a".to_string(), vec!["outer".to_string()]),
                ("b".to_string(), vec!["outer".to_string(), "1".to_string()]),
                ("c".to_string(), vec![]),
            ]
        );
        assert_eq!(
            steps[1].group[1].condition,
            crate::StepCondition::Failure,
            "the inner group's own condition travels with its frame"
        );
        assert_eq!(
            steps[1].condition,
            crate::StepCondition::Failure,
            "a default step inherits its group's `if: failure`"
        );
        assert_eq!(steps[0].condition, crate::StepCondition::Success);
        assert!(steps.iter().all(|step| !is_group(step)));
        assert_eq!(
            definition.criteria[0].checks[0].worst_case_step_count(),
            Ok(3)
        );
    }

    #[test]
    fn lifecycle_group_conditions_and_loop_body_leaves_are_reference_validated() {
        let bad_condition = "verification: x\nsetup:\n  - if: $setup.nope.outputs.n > 0\n    steps:\n      - uses: cli/invoke\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        let errors =
            crate::validate(&VerificationDefinition::from_yaml_str(bad_condition).unwrap())
                .expect_err("unknown setup reference in a group condition");
        assert!(
            errors.iter().any(|e| e.to_string().contains("nope")),
            "{errors:?}"
        );
        let bad_leaf = "verification: x\ninputs:\n  rows: { type: array }\nsetup:\n  - for_each: $inputs.rows\n    max: 2\n    as: row\n    steps:\n      - steps:\n          - uses: cli/invoke\n            with: { command: [echo, $ghost] }\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n";
        let errors = crate::validate(&VerificationDefinition::from_yaml_str(bad_leaf).unwrap())
            .expect_err("out-of-scope loop variable in a grouped loop-body step");
        assert!(
            errors.iter().any(|e| e.to_string().contains("ghost")),
            "{errors:?}"
        );
    }

    #[test]
    fn group_with_uses_is_rejected_with_a_location() {
        rejected(
            "          - uses: cli/invoke\n            steps:\n              - { uses: cli/invoke }\n",
            "",
            "no `uses:`",
        );
    }

    #[test]
    fn with_on_a_plain_group_is_rejected_with_a_location() {
        rejected(
            "          - with: { command: [echo] }\n            steps:\n              - { uses: cli/invoke }\n",
            "",
            "`with:` is not allowed on a step group",
        );
    }

    #[test]
    fn duplicate_ids_across_nesting_levels_are_rejected_with_a_location() {
        rejected(
            "          - id: g\n            steps:\n              - { id: dup, uses: cli/invoke }\n          - { id: dup, uses: cli/invoke }\n",
            "",
            "duplicate step id `dup`",
        );
        rejected(
            "          - id: same\n            steps:\n              - { id: same, uses: cli/invoke }\n",
            "",
            "duplicate step id `same`",
        );
    }

    #[test]
    fn referencing_a_group_id_is_rejected_with_a_location() {
        let definition = VerificationDefinition::from_yaml_str(
            "verification: groups\ncriteria:\n  - id: AC-1\n    description: groups\n    checks:\n      - id: AC-1.1\n        steps:\n          - id: prepare\n            steps:\n              - { id: mk, uses: cli/invoke }\n        assertions:\n          - $steps.prepare.outputs.stdout == \"ok\"\n",
        )
        .expect("parse");
        let errors = crate::validate(&definition).expect_err("a group has no outputs");
        assert!(
            errors.iter().any(|error| matches!(
                error,
                ValidationError::InvalidStepGroup { message, location: Some(_) }
                    if message.contains("`$steps.prepare` names a step group")
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn for_each_anywhere_under_a_for_each_is_rejected_through_groups() {
        rejected(
            "          - for_each: $inputs.rows\n            max: 2\n            steps:\n              - steps:\n                  - for_each: $inputs.cols\n                    max: 2\n                    uses: cli/invoke\n",
            "inputs:\n  rows: { type: array }\n  cols: { type: array }\n",
            "under another `for_each:`",
        );
    }

    #[test]
    fn a_group_may_sit_inside_a_loop_body() {
        let definition = parse(
            "          - for_each: $inputs.rows\n            max: 2\n            steps:\n              - steps:\n                  - { uses: cli/invoke }\n                  - { uses: cli/invoke }\n",
            "inputs:\n  rows: { type: array }\n",
        );
        crate::validate(&definition).expect("group inside a loop body validates");
        let mut lowered = definition;
        crate::flows::validate_and_expand(&mut lowered).expect("expands");
        let wrapper = &lowered.criteria[0].checks[0].steps[0];
        assert_eq!(wrapper.for_each_body.len(), 2);
        assert_eq!(lowered.criteria[0].checks[0].worst_case_step_count(), Ok(4));
    }

    #[test]
    fn group_session_fills_inner_ui_steps_and_an_inner_declaration_wins() {
        let mut definition = VerificationDefinition::from_yaml_str(
            "verification: sessions\ncriteria:\n  - id: AC-1\n    description: s\n    checks:\n      - id: AC-1.1\n        sessions: { a: ~, b: ~ }\n        steps:\n          - session: a\n            steps:\n              - { uses: ui/click, with: { role: button, name: Go } }\n              - { uses: ui/click, session: b, with: { role: button, name: Go } }\n        assertions: [\"true\"]\n",
        )
        .expect("parse");
        crate::flows::validate_and_expand(&mut definition).expect("expands");
        let steps = &definition.criteria[0].checks[0].steps;
        assert_eq!(steps[0].session.as_deref(), Some("a"));
        assert_eq!(steps[1].session.as_deref(), Some("b"));
    }

    #[test]
    fn a_group_around_a_call_expands_the_flow_inside_the_group() {
        let mut definition = VerificationDefinition::from_yaml_str(
            "verification: calls\nflows:\n  say:\n    steps:\n      - steps:\n          - { id: one, uses: cli/invoke }\n          - { id: two, uses: cli/invoke }\ncriteria:\n  - id: AC-1\n    description: c\n    checks:\n      - id: AC-1.1\n        steps:\n          - id: wrap\n            steps:\n              - { call: say }\n        assertions: [\"true\"]\n",
        )
        .expect("parse");
        crate::flows::validate_and_expand(&mut definition).expect("expands");
        let steps = &definition.criteria[0].checks[0].steps;
        assert_eq!(steps.len(), 2);
        for step in steps {
            let labels: Vec<_> = step.group.iter().map(|g| g.label.as_str()).collect();
            assert_eq!(
                labels,
                ["wrap", "0"],
                "call-site group wraps the flow's own group"
            );
        }
        let keys: BTreeSet<_> = steps
            .iter()
            .flat_map(|step| step.group.iter().map(|g| g.key))
            .collect();
        assert_eq!(keys.len(), 2, "both steps share both groups");
    }
}
