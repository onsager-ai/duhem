//! Structural rules for non-verdict score steps.
use std::collections::HashSet;

use crate::source::SourcePathSegment as Segment;
use crate::{Expr, PathRoot, Step, StepCondition, ValidationError, VerificationDefinition};

pub const ACTION: &str = "llm/score";

pub fn is_score(step: &Step) -> bool {
    step.uses.as_deref() == Some(ACTION)
}

/// The key is a whole reference to one input, never a literal or expression result.
fn key_input(step: &Step) -> Option<String> {
    let raw = step.with.get("api_key")?.as_str()?;
    let Expr::Path(path) = crate::expr::parse(raw).ok()? else {
        return None;
    };
    match (path.root, path.segments()) {
        (PathRoot::Inputs, [name]) => Some(name.clone()),
        _ => None,
    }
}

/// A declared source of data sent with the rubric.
pub enum ObservationSource<'a> {
    StepOutput,
    File(&'a str),
    Evidence(&'a str),
}

pub fn observation_source(value: &serde_yml::Value) -> Option<ObservationSource<'_>> {
    if let Some(raw) = value.as_str() {
        let Expr::Path(path) = crate::expr::parse(raw).ok()? else {
            return None;
        };
        let segments = path.segments();
        return (path.root == PathRoot::Steps && segments.len() >= 3 && segments[1] == "outputs")
            .then_some(ObservationSource::StepOutput);
    }
    let map = value.as_mapping()?;
    if map.len() != 1 {
        return None;
    }
    let (kind, value) = map.iter().next()?;
    let raw = value.as_str()?;
    match kind.as_str()? {
        "file" if !raw.is_empty() && !crate::expr::is_expression(raw) => {
            Some(ObservationSource::File(raw))
        }
        "evidence"
            if raw.len() == 64
                && raw
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
        {
            Some(ObservationSource::Evidence(raw))
        }
        _ => None,
    }
}

fn valid_range(with: &serde_yml::Value) -> bool {
    let Some(range) = with.get("range").and_then(serde_yml::Value::as_mapping) else {
        return false;
    };
    let min = range
        .get(serde_yml::Value::String("min".into()))
        .and_then(serde_yml::Value::as_f64);
    let max = range
        .get(serde_yml::Value::String("max".into()))
        .and_then(serde_yml::Value::as_f64);
    matches!((min, max), (Some(min), Some(max)) if range.len() == 2 && min.is_finite() && max.is_finite() && min < max)
}

pub(crate) fn validate(def: &VerificationDefinition, errors: &mut Vec<ValidationError>) {
    for block in crate::step_walk::step_blocks(def) {
        let flow_template = matches!(block.path.as_slice(), [Segment::Key(root), _, Segment::Key(field), ..] if root == "flows" && field == "steps");
        for (index, step) in block
            .steps
            .iter()
            .enumerate()
            .filter(|(_, step)| is_score(step))
        {
            let mut reject = |reason| {
                errors.push(ValidationError::InvalidFlow {
                    message: format!("{}: `{ACTION}` {reason}", block.step_site(index)),
                })
            };
            if block.check.is_none() && !flow_template {
                reject("is only allowed in check bodies");
            }
            if step.id.as_deref().is_none_or(str::is_empty) {
                reject("requires an `id`");
            }
            if !step.outputs.is_empty() || !step.secret_outputs.is_empty() {
                reject("does not expose outputs to expressions or output bindings");
            }
            if !flow_template && !valid_range(&step.with) {
                reject("requires numeric `range` bounds with `min < max`");
            }
            if !flow_template
                && !step
                    .with
                    .get("observations")
                    .and_then(serde_yml::Value::as_sequence)
                    .is_some_and(|sources| {
                        sources
                            .iter()
                            .all(|source| observation_source(source).is_some())
                    })
            {
                reject(
                    "requires an `observations` list of step outputs, `{file: path}`, or `{evidence: sha256}`",
                );
            }
            if !flow_template
                && !key_input(step)
                    .and_then(|key| def.inputs.get(&key))
                    .is_some_and(|decl| decl.secret)
            {
                reject("requires `api_key` to reference a declared `secret: true` input");
            }
        }
    }
    for criterion in &def.criteria {
        for check in &criterion.checks {
            let steps = check.steps.iter().flat_map(|step| {
                std::iter::once(step)
                    .chain(step.steps.iter().flatten())
                    .chain(&step.for_each_body)
            });
            let ids: HashSet<_> = steps
                .clone()
                .filter(|step| is_score(step))
                .filter_map(|step| step.id.as_deref())
                .collect();
            if ids.is_empty() {
                continue;
            }
            if check.assertions.is_empty()
                && steps
                    .clone()
                    .filter(|step| step.uses.is_some())
                    .all(is_score)
            {
                errors.push(ValidationError::NothingToJudge {
                    criterion: criterion.id.clone(),
                    check: check.id.clone(),
                });
            }
            let mut reject = |expr: &Expr| {
                reject_references(expr, &ids, &format!("check `{}`", check.id), errors)
            };
            for assertion in &check.assertions {
                assertion.walk_exprs(|expr| reject(&expr.parsed));
            }
            for step in steps {
                visit_step_exprs(step, &mut reject);
            }
        }
    }
    for (name, flow) in &def.flows {
        let steps = crate::step_groups::tree(&flow.steps);
        let ids: HashSet<_> = steps
            .iter()
            .filter(|step| is_score(step))
            .filter_map(|step| step.id.as_deref())
            .collect();
        if ids.is_empty() {
            continue;
        }
        let mut reject =
            |expr: &Expr| reject_references(expr, &ids, &format!("flow `{name}`"), errors);
        for step in steps
            .into_iter()
            .chain(crate::step_groups::tree(&flow.cleanup))
        {
            visit_step_exprs(step, &mut reject);
        }
        for raw in flow.outputs.values() {
            if let Ok(expr) = crate::expr::parse(raw) {
                reject(&expr);
            }
        }
    }
}

fn reject_references(
    expr: &Expr,
    ids: &HashSet<&str>,
    site: &str,
    errors: &mut Vec<ValidationError>,
) {
    expr.walk_paths(|path| {
        if path.root == PathRoot::Steps
            && let Some(id) = path.segments().first()
            && ids.contains(id.as_str())
        {
            errors.push(ValidationError::InvalidFlow {
                message: format!("{site}: cannot reference score step `{id}`"),
            });
        }
    });
}

fn visit_step_exprs(step: &Step, visit: &mut impl FnMut(&Expr)) {
    for condition in
        std::iter::once(&step.condition).chain(step.group.iter().map(|frame| &frame.condition))
    {
        if let StepCondition::Expr(expr) = condition {
            visit(&expr.parsed);
        }
    }
    if let Some(expr) = &step.for_each {
        visit(&expr.parsed);
    }
    crate::source::walk_with_refs(&step.with, &mut Vec::new(), &mut |expr, _, _| visit(expr));
}

/// Required-input checks may ignore an absent input used as a scoring key
/// when no mechanical part of the definition references it.
pub fn score_only_keys(def: &VerificationDefinition) -> HashSet<String> {
    let blocks = crate::step_walk::step_blocks(def);
    let mut keys: HashSet<_> = blocks
        .iter()
        .filter(|block| block.check.is_some())
        .flat_map(|block| block.steps)
        .filter(|step| is_score(step))
        .filter_map(key_input)
        .collect();
    if keys.is_empty() {
        return keys;
    }
    let mut remove = |expr: &Expr| {
        expr.walk_paths(|path| {
            if path.root == PathRoot::Inputs
                && let Some(name) = path.segments().first()
            {
                keys.remove(name);
            }
        })
    };
    for block in blocks {
        for step in block.steps.iter().filter(|step| !is_score(step)) {
            visit_step_exprs(step, &mut remove);
        }
    }
    for criterion in &def.criteria {
        for check in &criterion.checks {
            for assertion in &check.assertions {
                assertion.walk_exprs(|expr| remove(&expr.parsed));
            }
            for raw in check.session.iter().flatten() {
                if let Ok(expr) = crate::expr::parse(raw) {
                    remove(&expr);
                }
            }
            for expr in check
                .sessions
                .iter()
                .flat_map(|sessions| sessions.values().flatten())
            {
                remove(&expr.parsed);
            }
        }
    }
    // These surfaces contain typed configuration rather than Steps.
    for config in [
        serde_yml::to_value(&def.pages),
        serde_yml::to_value(&def.provision),
        serde_yml::to_value(&def.native),
    ]
    .into_iter()
    .flatten()
    {
        crate::source::walk_with_refs(&config, &mut Vec::new(), &mut |expr, _, _| remove(expr));
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "verification: test\ninputs:\n  key: {type: string, secret: true}\ncriteria:\n  - id: AC-1\n    description: mechanical\n    checks:\n      - id: AC-1.1\n";
    const STEP: &str = "          - id: rating\n            uses: llm/score\n            with: {api_key: $inputs.key, observations: [], range: {min: 0, max: 10}}\n";
    const ASSERTION: &str = "        assertions:\n          - '1 == 1'\n";

    fn errors(src: &str) -> String {
        let def = VerificationDefinition::from_yaml_str(src).unwrap();
        crate::validate(&def)
            .unwrap_err()
            .into_iter()
            .map(|err| err.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn score_paths_are_forbidden_even_inside_default() {
        let assertion = format!(
            "{HEADER}        steps:\n{STEP}        assertions:\n          - '$steps.rating.outputs.score == 1'\n"
        );
        assert!(errors(&assertion).contains("cannot reference score step `rating`"));
        for following in [
            "          - uses: cli/invoke\n            if: $steps.rating.outputs.score == 1\n",
            "          - uses: cli/invoke\n            with: {command: '$runtime.default($steps.rating.outputs.score, 1)'}\n",
            "          - for_each: $steps.rating.outputs.score\n            as: item\n            max: 1\n            steps: [{uses: cli/invoke}]\n",
        ] {
            let src = format!("{HEADER}        steps:\n{STEP}{following}{ASSERTION}");
            assert!(errors(&src).contains("cannot reference score step `rating`"));
        }
    }

    #[test]
    fn score_requires_id_secret_and_a_mechanical_claim() {
        let src = format!("{HEADER}        steps:\n{STEP}");
        assert!(
            matches!(crate::validate(&VerificationDefinition::from_yaml_str(&src).unwrap()), Err(errors) if errors.iter().any(|err| matches!(err, ValidationError::NothingToJudge { .. })))
        );
        let src = format!("{src}{ASSERTION}");
        assert!(
            errors(&src.replace("- id: rating\n            uses:", "- uses:"))
                .contains("requires an `id`")
        );
        assert!(errors(&src.replace("$inputs.key", "literal")).contains("secret: true"));
    }

    #[test]
    fn lifecycle_cannot_score() {
        for phase in ["setup", "teardown"] {
            let src = format!(
                "{phase}: [{{uses: llm/score, id: rating}}]\n{HEADER}        steps:\n{STEP}{ASSERTION}"
            );
            assert!(errors(&src).contains("only allowed in check bodies"));
        }
        let src = format!(
            "flows:\n  task:\n    steps: [{{uses: cli/invoke}}]\n    cleanup: [{{uses: llm/score, id: rating}}]\n{HEADER}        steps:\n{STEP}{ASSERTION}"
        );
        assert!(errors(&src).contains("only allowed in check bodies"));
        assert!(VerificationDefinition::from_yaml_str(&format!("provision: [{{uses: llm/score, id: rating}}]\n{HEADER}        steps:\n{STEP}{ASSERTION}")).is_err());
    }

    #[test]
    fn mechanical_use_keeps_key_required() {
        let src = format!("{HEADER}        steps:\n{STEP}{ASSERTION}");
        let def = VerificationDefinition::from_yaml_str(&src).unwrap();
        assert_eq!(score_only_keys(&def), HashSet::from(["key".to_string()]));
        let def = VerificationDefinition::from_yaml_str(
            &src.replace("'1 == 1'", r#""$inputs.key == 'expected'""#),
        )
        .unwrap();
        assert!(score_only_keys(&def).is_empty());
    }
}
