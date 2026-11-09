//! Step-group validation (spec #614).
//!
//! A step that declares `steps:` without `for_each:` is a group: a scope and
//! label over its children, lowered into the enclosing sequence by flow
//! expansion. These rules run on the authored tree, before lowering, because
//! the lowered sequence no longer remembers the group structure.

use std::collections::HashSet;

use crate::source::SourcePathSegment as Segment;
use crate::step_walk::step_blocks;
use crate::{PathRoot, Step, ValidationError, VerificationDefinition};

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
