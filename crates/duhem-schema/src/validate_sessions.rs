//! Named browser-context declarations and selectors (spec #508).
use crate::source::{SourcePathSegment as S, check_path};
use crate::{Check, SourceLocation, Step, ValidationError, VerificationDefinition};

const DEFAULT_MAX_SESSIONS: usize = 4;

pub(crate) fn validate(v: &VerificationDefinition, errors: &mut Vec<ValidationError>) {
    for (ci, criterion) in v.criteria.iter().enumerate() {
        for (ki, check) in criterion.checks.iter().enumerate() {
            let path = check_path(ci, ki, "sessions");
            let location = v.source_map.node_location(&path);
            let site = format!("criterion `{}` / check `{}`", criterion.id, check.id);
            if let Some(sessions) = &check.sessions {
                if check.session.is_some() {
                    error(
                        errors,
                        &site,
                        "`session:` and `sessions:` are mutually exclusive",
                        location,
                    );
                }
                let max = v.max_sessions.unwrap_or(DEFAULT_MAX_SESSIONS);
                if sessions.len() > max {
                    error(
                        errors,
                        &site,
                        &format!(
                            "`sessions:` declares {} contexts, exceeding defaults.max_sessions ({max})",
                            sessions.len()
                        ),
                        location,
                    );
                }
                for name in sessions.keys() {
                    if name.is_empty() || name.trim() != name || name.starts_with('$') {
                        error(
                            errors,
                            &site,
                            "`sessions:` keys must be non-empty bare names, never `$` expressions",
                            location,
                        );
                    }
                }
            }
            for (i, step) in check.steps.iter().enumerate() {
                let mut path = check_path(ci, ki, "steps");
                path.push(S::index(i));
                validate_step(v, step, Some(check), path, &site, errors);
            }
            for field in ["setup", "teardown"] {
                let steps = if field == "setup" {
                    &check.setup
                } else {
                    &check.teardown
                };
                for (i, step) in steps.iter().enumerate() {
                    let mut path = check_path(ci, ki, field);
                    path.push(S::index(i));
                    validate_step(v, step, Some(check), path, &site, errors);
                }
            }
        }
        for (field, steps) in [
            ("setup", &criterion.setup),
            ("teardown", &criterion.teardown),
        ] {
            for (i, step) in steps.iter().enumerate() {
                validate_step(
                    v,
                    step,
                    None,
                    vec![S::key("criteria"), S::index(ci), S::key(field), S::index(i)],
                    field,
                    errors,
                );
            }
        }
    }
    for (field, steps) in [("setup", &v.setup), ("teardown", &v.teardown)] {
        for (i, step) in steps.iter().enumerate() {
            validate_step(
                v,
                step,
                None,
                vec![S::key(field), S::index(i)],
                field,
                errors,
            );
        }
    }
    for (name, fixture) in &v.fixtures {
        for (field, steps) in [("up", &fixture.up), ("down", &fixture.down)] {
            for (i, step) in steps.iter().enumerate() {
                validate_step(
                    v,
                    step,
                    None,
                    vec![S::key("fixtures"), S::key(name), S::key(field), S::index(i)],
                    field,
                    errors,
                );
            }
        }
    }
}

fn validate_step(
    v: &VerificationDefinition,
    step: &Step,
    check: Option<&Check>,
    mut path: Vec<S>,
    site: &str,
    errors: &mut Vec<ValidationError>,
) {
    if step.for_each.is_some() {
        for (index, inner) in step.for_each_body.iter().enumerate() {
            let mut inner_path = path.clone();
            inner_path.extend([S::key("steps"), S::index(index)]);
            validate_step(v, inner, check, inner_path, site, errors);
        }
    }
    let ui = step
        .uses
        .as_deref()
        .is_some_and(|uses| uses.starts_with("ui/"));
    path.push(S::key(if step.session.is_some() {
        "session"
    } else {
        "uses"
    }));
    let location = if let [
        S::Key(_),
        S::Index(ci),
        S::Key(_),
        S::Index(ki),
        S::Key(field),
        S::Index(si),
        ..,
    ] = path.as_slice()
    {
        if field == "steps" {
            step.session
                .as_deref()
                .or(step.uses.as_deref())
                .and_then(|raw| {
                    v.source_map
                        .step_with_location(step, *ci, *ki, *si, &path, raw)
                })
        } else {
            v.source_map.node_location(&path)
        }
    } else {
        v.source_map.node_location(&path)
    };
    if let Some(name) = &step.session {
        let message = if name.trim_start().starts_with('$') {
            Some("step-level `session:` is a bare context name, never an expression; declare acquired state in check `sessions:`".to_string())
        } else if !ui {
            Some("`session:` is only valid on browser-driving `ui/*` steps".to_string())
        } else if !check
            .and_then(|check| check.sessions.as_ref())
            .is_some_and(|sessions| sessions.contains_key(name))
        {
            Some(format!(
                "undeclared session `{name}`; declare it in the check's `sessions:` map"
            ))
        } else {
            None
        };
        if let Some(message) = message {
            error(errors, site, &message, location);
        }
    } else if ui && check.is_some_and(|check| check.sessions.is_some()) {
        error(
            errors,
            site,
            "every browser-driving step must name a `session:` when `sessions:` is declared",
            location,
        );
    }
}

fn error(
    errors: &mut Vec<ValidationError>,
    site: &str,
    message: &str,
    location: Option<SourceLocation>,
) {
    errors.push(ValidationError::InvalidNamedSession {
        message: format!("{site}: {message}"),
        location,
    });
}
