//! Validation for the lifecycle authoring surface — `fixtures:` (#449)
//! and value-based `if:` on lifecycle steps (#440).
//!
//! Split out of `validate.rs` alongside the error vocabulary (#411).
//! Both checks concern the same surface — the `setup:` / `teardown:` /
//! `fixtures:` lifecycle — and are self-contained walks over it, so
//! they move together rather than growing the structural validator.

use std::collections::{HashMap, HashSet};

use crate::expr::PathRoot;
use crate::source::SourcePathSegment;
use crate::step::{Step, StepCondition};
use crate::validate::{effective_outputs, step_label};
use crate::validate_error::ValidationError;
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
    index: usize,
    step: &Step,
    preceding: &HashMap<&str, HashSet<String>>,
    errs: &mut Vec<ValidationError>,
) {
    let StepCondition::Expr(expr) = &step.condition else {
        return;
    };
    let source_path = [
        SourcePathSegment::key(phase),
        SourcePathSegment::index(index),
        SourcePathSegment::key("if"),
    ];
    let location = definition
        .source_map
        .scalar_location(&source_path, &expr.raw);
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
