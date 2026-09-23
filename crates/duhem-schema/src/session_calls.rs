//! Validate call selectors before expansion can erase an empty or non-UI call.
use crate::source::SourcePathSegment as S;
use crate::{Check, Step, VerificationDefinition};

pub(crate) fn validate(v: &VerificationDefinition) -> Vec<String> {
    let mut errors = Vec::new();
    for (field, steps) in [("setup", &v.setup), ("teardown", &v.teardown)] {
        walk(v, steps, None, &[S::key(field)], 0, &mut errors);
    }
    for (ci, criterion) in v.criteria.iter().enumerate() {
        for (field, steps) in [
            ("setup", &criterion.setup),
            ("teardown", &criterion.teardown),
        ] {
            walk(
                v,
                steps,
                None,
                &[S::key("criteria"), S::index(ci), S::key(field)],
                0,
                &mut errors,
            );
        }
        for (ki, check) in criterion.checks.iter().enumerate() {
            for (field, steps) in [
                ("steps", &check.steps),
                ("setup", &check.setup),
                ("teardown", &check.teardown),
            ] {
                walk(
                    v,
                    steps,
                    Some(check),
                    &crate::source::check_path(ci, ki, field),
                    0,
                    &mut errors,
                );
            }
            for name in &check.needs {
                if let Some(fixture) = v.fixtures.get(name) {
                    for (field, steps) in [("up", &fixture.up), ("down", &fixture.down)] {
                        walk(
                            v,
                            steps,
                            Some(check),
                            &[S::key("fixtures"), S::key(name), S::key(field)],
                            0,
                            &mut errors,
                        );
                    }
                }
            }
        }
    }
    errors
}

fn walk(
    v: &VerificationDefinition,
    steps: &[Step],
    check: Option<&Check>,
    path: &[S],
    depth: usize,
    errors: &mut Vec<String>,
) {
    if depth > crate::flows::MAX_FLOW_DEPTH {
        return;
    }
    for (i, step) in steps.iter().enumerate() {
        let mut site = path.to_vec();
        site.push(S::index(i));
        if let Some(call) = &step.call {
            if let Some(name) = &step.session
                && !check
                    .and_then(|c| c.sessions.as_ref())
                    .is_some_and(|sessions| sessions.contains_key(name))
            {
                let mut field = site.clone();
                field.push(S::key("session"));
                let location = v
                    .source_map
                    .node_location(&field)
                    .map(|loc| format!(" at line {}, column {}", loc.line, loc.column))
                    .unwrap_or_default();
                errors.push(format!("call `{call}` session `{name}` must name a declared context in a named-sessions check{location}"));
            }
            if let Some(flow) = v.flows.get(call) {
                walk(
                    v,
                    &flow.steps,
                    check,
                    &[S::key("flows"), S::key(call), S::key("steps")],
                    depth + 1,
                    errors,
                );
            }
        }
        if let Some(body) = &step.steps {
            site.push(S::key("steps"));
            walk(v, body, check, &site, depth + 1, errors);
        }
    }
}
