//! Check references and assertions, including seed timing.
use super::*;

pub(super) fn validate_check(
    c: &Criterion,
    criterion_index: usize,
    ch: &Check,
    check_index: usize,
    definition: &DefinitionScope<'_>,
    errs: &mut Vec<ValidationError>,
) {
    let DefinitionScope {
        inputs,
        pages,
        flows,
        source_map,
        setup_outputs,
        outputs_for,
    } = *definition;
    let check_steps: Vec<(&Step, Option<&str>)> = ch
        .steps
        .iter()
        .flat_map(|s| {
            std::iter::once((s, None)).chain(
                (if s.for_each_body.is_empty() {
                    s.steps.as_deref().unwrap_or(&[])
                } else {
                    &s.for_each_body
                })
                .iter()
                .map(move |body| (body, s.as_binding.as_deref())),
            )
        })
        .collect();
    let source_context_matches =
        source_map.check_context_matches(criterion_index, &c.id, check_index, &ch.id);
    if let Err(error) = ch.worst_case_step_count() {
        errs.push(ValidationError::UncomputableStepCount {
            criterion: c.id.clone(),
            check: ch.id.clone(),
            reason: error.to_string(),
        });
    }
    // A check with neither assertions nor steps can never produce a
    // verdict (the judge would see an empty aggregation). With steps
    // but no assertions the schema layer accepts — whether one of the
    // steps is a judging action (implicit judgment, spec #253) is a
    // catalog question the contract-aware CLI layer answers.
    if ch.assertions.is_empty() && ch.steps.is_empty() {
        errs.push(ValidationError::NothingToJudge {
            criterion: c.id.clone(),
            check: ch.id.clone(),
        });
    }

    let mut step_outputs: HashMap<&str, HashSet<String>> = HashMap::new();
    let mut seen_step_ids: HashSet<&str> = HashSet::new();

    for (idx, &(s, _)) in check_steps.iter().enumerate() {
        if matches!(s.condition, StepCondition::Expr(_)) {
            let path = [
                SourcePathSegment::key("criteria"),
                SourcePathSegment::index(criterion_index),
                SourcePathSegment::key("checks"),
                SourcePathSegment::index(check_index),
                SourcePathSegment::key("steps"),
                SourcePathSegment::index(idx),
                SourcePathSegment::key("if"),
            ];
            let raw = match &s.condition {
                StepCondition::Expr(e) => e.raw.as_str(),
                _ => unreachable!(),
            };
            errs.push(ValidationError::CheckStepConditionUnavailable {
                criterion: c.id.clone(),
                check: ch.id.clone(),
                location: source_context_matches
                    .then(|| source_map.scalar_location(&path, raw))
                    .flatten(),
            });
        }
        let contract_outputs = s.uses.as_deref().map(outputs_for).unwrap_or_default();
        check_secret_paths(
            s,
            &contract_outputs,
            &format!("criterion `{}` / check `{}`", c.id, ch.id),
            &step_label(s, idx),
            errs,
        );
        if let Some(id) = &s.id {
            if !seen_step_ids.insert(id.as_str()) {
                errs.push(ValidationError::DuplicateStepId {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    id: id.clone(),
                });
            }
            let outputs = if let Some(call) = s.call.as_deref() {
                flows
                    .get(call)
                    .map(|flow| flow.outputs.keys().cloned().collect())
                    .unwrap_or_default()
            } else {
                effective_outputs(s, outputs_for)
            };
            step_outputs.insert(id.as_str(), outputs);
        }
    }

    // Declared but not check-body-addressable (#441 Part B) — see
    // `PathScope::out_of_scope_setup_ids`.
    let out_of_scope_setup_ids: HashSet<&str> = c
        .setup
        .iter()
        .chain(ch.setup.iter())
        .filter_map(|s| s.id.as_deref())
        .collect();

    let scope = PathScope {
        active_loop: None,
        c,
        ch,
        step_outputs: &step_outputs,
        setup_outputs,
        out_of_scope_setup_ids: &out_of_scope_setup_ids,
        inputs,
        pages,
    };

    // A browser session is acquired state, not an inline fixture. Only
    // a whole-string path reference is accepted: literals and runtime
    // calls could fabricate auth state at the authoring boundary. Once
    // parsed, the ordinary reference checker supplies the same
    // undeclared input/setup diagnostics as `with:` (#134).
    let mut session_outputs = setup_outputs.clone();
    for step in &c.setup {
        if let Some(id) = step.id.as_deref() {
            session_outputs.insert(id, effective_outputs(step, outputs_for));
        }
    }
    let own_setup_ids = ch.setup.iter().filter_map(|s| s.id.as_deref()).collect();
    let session_scope = PathScope {
        setup_outputs: &session_outputs,
        out_of_scope_setup_ids: &own_setup_ids,
        ..scope
    };
    for (name, raw) in ch
        .session
        .iter()
        .flatten()
        .map(|raw| (None, raw.as_str()))
        .chain(
            ch.sessions
                .iter()
                .flat_map(|sessions| sessions.iter())
                .filter_map(|(name, value)| {
                    value.as_ref().map(|expr| (Some(name), expr.raw.as_str()))
                }),
        )
    {
        let location = source_context_matches
            .then(|| {
                let mut path = crate::source::check_path(
                    criterion_index,
                    check_index,
                    if name.is_some() {
                        "sessions"
                    } else {
                        "session"
                    },
                );
                if let Some(name) = name {
                    path.push(SourcePathSegment::key(name));
                }
                source_map.scalar_location(&path, raw)
            })
            .flatten();
        match crate::expr::parse(raw) {
            Ok(Expr::Path(path)) => {
                if matches!(
                    path.root,
                    PathRoot::Steps | PathRoot::Fixture | PathRoot::Loop
                ) {
                    errs.push(ValidationError::InvalidSessionReference {
                        criterion: c.id.clone(),
                        check: ch.id.clone(),
                        value: raw.to_string(),
                        location,
                    });
                } else {
                    check_path(
                        &session_scope,
                        &path,
                        None,
                        raw,
                        &RefSite::Session,
                        location,
                        errs,
                    );
                }
            }
            _ => errs.push(ValidationError::InvalidSessionReference {
                criterion: c.id.clone(),
                check: ch.id.clone(),
                value: raw.to_string(),
                location,
            }),
        }
    }

    for (assertion_index, assertion) in ch.assertions.iter().enumerate() {
        assertion.walk_exprs(|expr_str| {
            let raw = expr_str.raw.as_str();
            let location = source_context_matches
                .then(|| {
                    source_map.assertion_location(
                        criterion_index,
                        check_index,
                        assertion_index,
                        assertion,
                        raw,
                    )
                })
                .flatten();
            walk_checkable_paths(&expr_str.parsed, &mut |p, arity| {
                check_path(&scope, p, arity, raw, &RefSite::Assertion, location, errs);
            });
        });
    }

    // Beyond assertions, a `$...` reference inside a step's `with:`
    // payload is the most common place an author writes one — and was
    // historically unscanned, so a typo'd or undeclared ref reached
    // the action as a literal `$...` string (#134). Walk every string
    // scalar in the (untyped) `with:` tree and resolve its references
    // against the same scope.
    for (idx, &(s, active_loop)) in check_steps.iter().enumerate() {
        if let Some(expr) = &s.for_each {
            walk_checkable_paths(&expr.parsed, &mut |p, arity| {
                check_path(
                    &scope,
                    p,
                    arity,
                    &expr.raw,
                    &RefSite::StepWith {
                        step: step_label(s, idx),
                    },
                    None,
                    errs,
                );
            });
        }
        let scope = PathScope {
            active_loop: active_loop.or(s.as_binding.as_deref()),
            ..scope
        };
        let site = RefSite::StepWith {
            step: step_label(s, idx),
        };
        let mut source_path = crate::source::check_path(criterion_index, check_index, "steps");
        source_path.push(SourcePathSegment::index(idx));
        source_path.push(SourcePathSegment::key("with"));
        crate::source::walk_with_strings(&s.with, &mut source_path, &mut |raw, path| {
            if raw.trim_start().starts_with('$')
                && let Err(error) = crate::expr::parse(raw)
            {
                let location = source_context_matches
                    .then(|| {
                        source_map.step_with_location(
                            s,
                            criterion_index,
                            check_index,
                            idx,
                            path,
                            raw,
                        )
                    })
                    .flatten();
                errs.push(ValidationError::InvalidWithExpression {
                    raw: raw.to_string(),
                    context: error.to_string(),
                    site: site.clone(),
                    location,
                });
            }
        });
        crate::source::walk_with_refs(&s.with, &mut source_path, &mut |expr, raw, path| {
            let location = source_context_matches
                .then(|| {
                    source_map.step_with_location(s, criterion_index, check_index, idx, path, raw)
                })
                .flatten();
            walk_checkable_paths(expr, &mut |p, arity| {
                check_path(&scope, p, arity, raw, &site, location, errs);
            });
        });
    }
}
