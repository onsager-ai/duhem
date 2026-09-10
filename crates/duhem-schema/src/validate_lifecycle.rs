//! Validation for the lifecycle authoring surface — `fixtures:` (#449)
//! and value-based `if:` on lifecycle steps (#440).
//!
//! Split out of `validate.rs` alongside the error vocabulary (#411).
//! Both checks concern the same surface — the `setup:` / `teardown:` /
//! `fixtures:` lifecycle — and are self-contained walks over it, so
//! they move together rather than growing the structural validator.

use std::collections::{HashMap, HashSet};

use crate::SourceLocation;
use crate::criterion::Criterion;
use crate::expr::PathRoot;
use crate::source::SourcePathSegment;
use crate::step::{Step, StepCondition};
use crate::validate::{check_with_expression_syntax, effective_outputs, step_label};
use crate::validate_error::{RefSite, ValidationError};
use crate::validate_runtime::walk_checkable_paths;
use crate::verification::VerificationDefinition;

pub(crate) fn validate_fixtures(
    v: &VerificationDefinition,
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    errs: &mut Vec<ValidationError>,
) {
    for (name, fixture) in &v.fixtures {
        if fixture.up.is_empty() {
            errs.push(ValidationError::EmptyFixturePhase {
                fixture: name.clone(),
                phase: "up",
            });
        }
        if fixture.down.is_empty() {
            errs.push(ValidationError::EmptyFixturePhase {
                fixture: name.clone(),
                phase: "down",
            });
        }
        let mut up_outputs: HashMap<&str, HashSet<String>> = HashMap::new();
        for (index, step) in fixture.up.iter().enumerate() {
            if !step.needs.is_empty() {
                let path = [
                    SourcePathSegment::key("fixtures"),
                    SourcePathSegment::key(name),
                    SourcePathSegment::key("up"),
                    SourcePathSegment::index(index),
                    SourcePathSegment::key("needs"),
                    SourcePathSegment::index(0),
                ];
                errs.push(ValidationError::FixtureStepNeeds {
                    fixture: name.clone(),
                    phase: "up",
                    step: step_label(step, index),
                    location: v.source_map.scalar_location(&path, &step.needs[0]),
                });
            }
            if let Some(id) = step.id.as_deref() {
                up_outputs.insert(id, effective_outputs(step, outputs_for));
            }
            let mut path = vec![
                SourcePathSegment::key("fixtures"),
                SourcePathSegment::key(name),
                SourcePathSegment::key("up"),
                SourcePathSegment::index(index),
                SourcePathSegment::key("with"),
            ];
            crate::validate::check_with_expression_syntax(
                &step.with,
                &mut path,
                crate::validate::RefSite::StepWith {
                    step: step_label(step, index),
                },
                &v.source_map,
                errs,
            );
            crate::source::walk_with_refs(&step.with, &mut path, &mut |expr, raw, source_path| {
                expr.walk_paths(|reference| {
                    if reference.root == PathRoot::Fixture {
                        errs.push(ValidationError::FixtureRefOutsideDown {
                            site: format!(
                                "fixture `{name}` up step `{}` with:",
                                step_label(step, index)
                            ),
                            location: v.source_map.scalar_location(source_path, raw),
                        });
                    }
                });
            });
        }
        for (index, step) in fixture.down.iter().enumerate() {
            if !step.needs.is_empty() {
                let path = [
                    SourcePathSegment::key("fixtures"),
                    SourcePathSegment::key(name),
                    SourcePathSegment::key("down"),
                    SourcePathSegment::index(index),
                    SourcePathSegment::key("needs"),
                    SourcePathSegment::index(0),
                ];
                errs.push(ValidationError::FixtureStepNeeds {
                    fixture: name.clone(),
                    phase: "down",
                    step: step_label(step, index),
                    location: v.source_map.scalar_location(&path, &step.needs[0]),
                });
            }
            let mut path = vec![
                SourcePathSegment::key("fixtures"),
                SourcePathSegment::key(name),
                SourcePathSegment::key("down"),
                SourcePathSegment::index(index),
                SourcePathSegment::key("with"),
            ];
            crate::validate::check_with_expression_syntax(
                &step.with,
                &mut path,
                crate::validate::RefSite::StepWith {
                    step: step_label(step, index),
                },
                &v.source_map,
                errs,
            );
            crate::source::walk_with_refs(&step.with, &mut path, &mut |expr, raw, source_path| {
                let location = v.source_map.scalar_location(source_path, raw);
                expr.walk_paths(|reference| {
                    if reference.root != PathRoot::Fixture {
                        return;
                    }
                    let segs = reference.segments();
                    let valid = segs.len() >= 4
                        && segs[0] == *name
                        && segs[2] == "outputs"
                        && up_outputs
                            .get(segs[1].as_str())
                            .is_some_and(|outputs| outputs.contains(&segs[3]));
                    if !valid {
                        errs.push(ValidationError::InvalidFixtureRef {
                            fixture: name.clone(),
                            down_step: step_label(step, index),
                            raw: raw.to_string(),
                            location,
                        });
                    }
                });
            });
        }
    }
}

pub(crate) fn validate_lifecycle_condition(
    definition: &VerificationDefinition,
    phase: &str,
    source_path: &[SourcePathSegment],
    step: &Step,
    preceding: &HashMap<&str, HashSet<String>>,
    errs: &mut Vec<ValidationError>,
) {
    let StepCondition::Expr(expr) = &step.condition else {
        return;
    };
    let location = definition
        .source_map
        .scalar_location(source_path, &expr.raw);
    walk_checkable_paths(&expr.parsed, &mut |path, arity| {
        let fail = |message: String, errs: &mut Vec<ValidationError>| {
            errs.push(ValidationError::InvalidStepCondition { message, location });
        };
        match path.root {
            PathRoot::Setup => {
                let segs = path.segments();
                if segs.len() < 3 || segs[1] != "outputs" {
                    fail(
                        format!(
                            "{phase} step condition `{}` has malformed `$setup` reference (expected `$setup.<step_id>.outputs.<output>`)",
                            expr.raw
                        ),
                        errs,
                    );
                } else if let Some(outputs) = preceding.get(segs[0].as_str()) {
                    if !outputs.contains(&segs[2]) {
                        fail(
                            format!(
                                "{phase} step condition `{}` references undeclared output `{}` on step `{}`",
                                expr.raw, segs[2], segs[0]
                            ),
                            errs,
                        );
                    }
                } else {
                    fail(
                        format!(
                            "{phase} step condition `{}` references undeclared or forward step `{}`",
                            expr.raw, segs[0]
                        ),
                        errs,
                    );
                }
            }
            PathRoot::Inputs => {
                let segs = path.segments();
                if segs.is_empty() || !definition.inputs.contains_key(&segs[0]) {
                    fail(
                        format!(
                            "{phase} step condition `{}` references undeclared input `{}`",
                            expr.raw,
                            segs.first().map(String::as_str).unwrap_or("")
                        ),
                        errs,
                    );
                }
            }
            PathRoot::Runtime => {
                crate::validate_runtime::check_runtime_path(
                    path,
                    arity,
                    &expr.raw,
                    &format!("{phase} step condition:"),
                    errs,
                );
            }
            PathRoot::Pages => {
                crate::validate_pages::check_page_path(
                    &definition.pages,
                    path,
                    arity,
                    &expr.raw,
                    &format!("{phase} step condition:"),
                    location,
                    errs,
                );
            }
            PathRoot::Steps => fail(
                format!(
                    "{phase} step condition `{}` must use `$setup` for earlier lifecycle-step outputs",
                    expr.raw
                ),
                errs,
            ),
            PathRoot::Fixture => fail(
                format!(
                    "{phase} step condition `{}` may not reference fixture outputs",
                    expr.raw
                ),
                errs,
            ),
            PathRoot::Env => {}
        }
    });
}

/// Validate one criterion- or check-level `setup:`/`teardown:` block
/// (#441 Part B) — the same walk the leaf-level pair gets (value-based
/// `if:` per #440 Tier 1, `with:` expression syntax, `$pages`/
/// `$runtime` reference checks, `$fixture` rejection), plus one rule
/// unique to nesting: a step id here must not collide with an id
/// already open in an *outer* lifecycle scope (`outer_ids`). At
/// runtime every lifecycle level publishes into the same flat
/// `$setup.<id>` namespace (so an outer teardown can read its own
/// setup's outputs with no new reference syntax); without this check
/// an inner block could silently clobber a value an outer teardown
/// still needs.
///
/// `source_path_prefix` is the path to the `Vec<Step>` itself (e.g.
/// `criteria/0/setup`); `label` is a short human prefix for messages
/// (e.g. `` criterion `AC-1` `` or `` criterion `AC-1` / check
/// `AC-1.1` ``). Returns this block's own step ids (id → referenceable
/// outputs) so the caller can fold them into `outer_ids` before
/// validating the next nested scope or this block's paired teardown.
#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_nested_lifecycle_block(
    v: &VerificationDefinition,
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    steps: &[Step],
    phase: &'static str,
    source_path_prefix: &[SourcePathSegment],
    label: &str,
    outer_ids: &HashMap<String, HashSet<String>>,
    errs: &mut Vec<ValidationError>,
) -> HashMap<String, HashSet<String>> {
    let mut own: HashMap<String, HashSet<String>> = HashMap::new();
    for (idx, step) in steps.iter().enumerate() {
        let step_name = step_label(step, idx);
        let mut step_path = source_path_prefix.to_vec();
        step_path.push(SourcePathSegment::index(idx));

        // `if:` resolution sees every id already in scope: outer
        // lifecycle blocks that ran before this one, plus this
        // block's own preceding steps.
        let preceding: HashMap<&str, HashSet<String>> = outer_ids
            .iter()
            .chain(own.iter())
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        let mut condition_path = step_path.clone();
        condition_path.push(SourcePathSegment::key("if"));
        validate_lifecycle_condition(v, phase, &condition_path, step, &preceding, errs);

        let mut with_path = step_path.clone();
        with_path.push(SourcePathSegment::key("with"));
        check_with_expression_syntax(
            &step.with,
            &mut with_path,
            RefSite::StepWith {
                step: step_name.clone(),
            },
            &v.source_map,
            errs,
        );
        crate::source::walk_with_refs(&step.with, &mut with_path, &mut |expr, raw, path| {
            let location = v.source_map.scalar_location(path, raw);
            walk_checkable_paths(expr, &mut |path, arity| {
                if path.root == PathRoot::Pages {
                    crate::validate_pages::check_page_path(
                        &v.pages,
                        path,
                        arity,
                        raw,
                        &format!("{label} {phase} step `{step_name}` with:"),
                        location,
                        errs,
                    );
                } else if path.root == PathRoot::Runtime {
                    crate::validate_runtime::check_runtime_path(
                        path,
                        arity,
                        raw,
                        &format!("{label} {phase} step `{step_name}` with:"),
                        errs,
                    );
                } else if path.root == PathRoot::Fixture {
                    errs.push(ValidationError::FixtureRefOutsideDown {
                        site: format!("{label} {phase} step `{step_name}` with:"),
                        location,
                    });
                }
            });
        });

        if let Some(id) = step.id.as_deref() {
            let mut id_path = step_path.clone();
            id_path.push(SourcePathSegment::key("id"));
            let location = v.source_map.scalar_location(&id_path, id);
            if outer_ids.contains_key(id) {
                errs.push(ValidationError::LifecycleStepIdCollision {
                    label: label.to_string(),
                    phase,
                    id: id.to_string(),
                    location,
                });
            } else if own.contains_key(id) {
                errs.push(ValidationError::DuplicateLifecycleStepId {
                    label: label.to_string(),
                    phase,
                    id: id.to_string(),
                    location,
                });
            } else {
                own.insert(id.to_string(), effective_outputs(step, outputs_for));
            }
        }
    }
    own
}

/// Validate one criterion's criterion-level `setup:`/`teardown:` and
/// every one of its checks' check-level `setup:`/`teardown:` (#441
/// Part B). Owns the full accumulation chain — leaf → criterion →
/// check — so `validate.rs`'s per-criterion loop stays a one-line
/// call; kept here because this file already owns the lifecycle
/// authoring surface and its shared error vocabulary (#411).
pub(crate) fn validate_criterion_and_check_lifecycle_hooks(
    v: &VerificationDefinition,
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    criterion_index: usize,
    c: &Criterion,
    leaf_ids: &HashMap<String, HashSet<String>>,
    errs: &mut Vec<ValidationError>,
) {
    let criterion_label = format!("criterion `{}`", c.id);
    let criterion_setup_ids = validate_nested_lifecycle_block(
        v,
        outputs_for,
        &c.setup,
        "setup",
        &[
            SourcePathSegment::key("criteria"),
            SourcePathSegment::index(criterion_index),
            SourcePathSegment::key("setup"),
        ],
        &criterion_label,
        leaf_ids,
        errs,
    );
    // Criterion `teardown:` sees leaf `setup:` plus this criterion's
    // own `setup:` — its immediate paired outer scope.
    let criterion_ids: HashMap<String, HashSet<String>> = leaf_ids
        .iter()
        .chain(criterion_setup_ids.iter())
        .map(|(id, outputs)| (id.clone(), outputs.clone()))
        .collect();
    validate_nested_lifecycle_block(
        v,
        outputs_for,
        &c.teardown,
        "teardown",
        &[
            SourcePathSegment::key("criteria"),
            SourcePathSegment::index(criterion_index),
            SourcePathSegment::key("teardown"),
        ],
        &criterion_label,
        &criterion_ids,
        errs,
    );

    for (check_index, check) in c.checks.iter().enumerate() {
        let check_label = format!("criterion `{}` / check `{}`", c.id, check.id);
        let check_setup_ids = validate_nested_lifecycle_block(
            v,
            outputs_for,
            &check.setup,
            "setup",
            &[
                SourcePathSegment::key("criteria"),
                SourcePathSegment::index(criterion_index),
                SourcePathSegment::key("checks"),
                SourcePathSegment::index(check_index),
                SourcePathSegment::key("setup"),
            ],
            &check_label,
            &criterion_ids,
            errs,
        );
        // Check `teardown:` sees leaf `setup:`, this check's criterion
        // `setup:`, and this check's own `setup:`.
        let check_ids: HashMap<String, HashSet<String>> = criterion_ids
            .iter()
            .chain(check_setup_ids.iter())
            .map(|(id, outputs)| (id.clone(), outputs.clone()))
            .collect();
        validate_nested_lifecycle_block(
            v,
            outputs_for,
            &check.teardown,
            "teardown",
            &[
                SourcePathSegment::key("criteria"),
                SourcePathSegment::index(criterion_index),
                SourcePathSegment::key("checks"),
                SourcePathSegment::index(check_index),
                SourcePathSegment::key("teardown"),
            ],
            &check_label,
            &check_ids,
            errs,
        );
    }
}

/// Resolve one `$setup.<step_id>.outputs.<output>` reference from a
/// check body (`with:`, `assertions:`, or `session:`). `setup_outputs`
/// is the leaf-level ids the check body may actually address;
/// `out_of_scope_setup_ids` is this check's own criterion-/check-level
/// `setup:` ids (#441 Part B) — declared, but not addressable from the
/// check body. A reference naming one of those gets
/// [`ValidationError::SetupStepOutOfScope`] instead of the
/// genuinely-undeclared `UnresolvedSetupStepRef`, so the message
/// doesn't send an author looking for a typo in a declaration that's
/// staring back at them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_setup_reference(
    criterion: &str,
    check: &str,
    segs: &[String],
    raw: &str,
    site: &RefSite,
    location: Option<SourceLocation>,
    setup_outputs: &HashMap<&str, HashSet<String>>,
    out_of_scope_setup_ids: &HashSet<&str>,
    errs: &mut Vec<ValidationError>,
) {
    // Leading `$setup.<step_id>.outputs.<output>` — same shape as
    // `$steps`; deeper segments navigate the value at runtime.
    if segs.len() < 3 || segs[1] != "outputs" {
        errs.push(ValidationError::MalformedSetupRef {
            criterion: criterion.to_string(),
            check: check.to_string(),
            raw: raw.to_string(),
            site: site.clone(),
            location,
        });
        return;
    }
    let step_id = segs[0].as_str();
    let output_name = segs[2].as_str();
    match setup_outputs.get(step_id) {
        None if out_of_scope_setup_ids.contains(step_id) => {
            errs.push(ValidationError::SetupStepOutOfScope {
                criterion: criterion.to_string(),
                check: check.to_string(),
                step: step_id.to_string(),
                raw: raw.to_string(),
                site: site.clone(),
                location,
            });
        }
        None => errs.push(ValidationError::UnresolvedSetupStepRef {
            criterion: criterion.to_string(),
            check: check.to_string(),
            step: step_id.to_string(),
            raw: raw.to_string(),
            site: site.clone(),
            location,
        }),
        Some(outputs) => {
            if !outputs.contains(output_name) {
                errs.push(ValidationError::UnresolvedSetupStepOutput {
                    criterion: criterion.to_string(),
                    check: check.to_string(),
                    step: step_id.to_string(),
                    output: output_name.to_string(),
                    raw: raw.to_string(),
                    site: site.clone(),
                    location,
                });
            }
        }
    }
}
