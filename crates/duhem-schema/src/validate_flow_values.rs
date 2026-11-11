//! Output contracts for value expressions in reusable flow bodies and cleanup.
use std::collections::{HashMap, HashSet};

use crate::source::SourcePathSegment as Segment;
use crate::{PathRoot, StepCondition, ValidationError, VerificationDefinition};

pub(crate) fn validate(
    definition: &VerificationDefinition,
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    errors: &mut Vec<ValidationError>,
) {
    let leaf: HashMap<_, _> = definition
        .setup
        .iter()
        .filter_map(|step| {
            Some((
                step.id.as_deref()?,
                crate::validate::effective_outputs(step, outputs_for),
            ))
        })
        .collect();
    for (name, flow) in &definition.flows {
        let mut body = HashMap::new();
        for (phase, steps) in [("steps", &flow.steps), ("cleanup", &flow.cleanup)] {
            for (index, step) in steps.iter().enumerate() {
                let site = format!("flow `{name}` {phase} step {index}");
                for (field, expression) in [
                    (
                        "if",
                        match &step.condition {
                            StepCondition::Expr(expr) => Some(expr),
                            _ => None,
                        },
                    ),
                    ("for_each", step.for_each.as_ref()),
                ] {
                    let Some(expression) = expression else {
                        continue;
                    };
                    let location = definition.source_map.scalar_location(
                        &[
                            Segment::key("flows"),
                            Segment::key(name),
                            Segment::key(phase),
                            Segment::index(index),
                            Segment::key(field),
                        ],
                        &expression.raw,
                    );
                    expression.parsed.walk_paths(|path| {
                        let outputs: Option<&HashMap<&str, HashSet<String>>> = match path.root {
                            PathRoot::Steps => Some(&body),
                            PathRoot::Setup => Some(&leaf),
                            _ => None,
                        };
                        let Some(outputs) = outputs else { return };
                        let segments = path.segments();
                        let valid = segments.len() >= 3 && segments[1] == "outputs"
                            && outputs.get(segments[0].as_str()).is_some_and(|names| names.contains(&segments[2]));
                        if !valid {
                            errors.push(ValidationError::InvalidStepCondition {
                                message: format!("{site} {field} `{}` references an undeclared, forward or out-of-scope step output", expression.raw),
                                location,
                            });
                        }
                    });
                }
                if let Some(id) = &step.id {
                    body.insert(
                        id.as_str(),
                        crate::validate::effective_outputs(step, outputs_for),
                    );
                }
            }
        }
    }
}
