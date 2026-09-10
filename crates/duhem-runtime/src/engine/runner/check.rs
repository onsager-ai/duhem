//! Sequential check execution with context-specific evidence.
use super::*;

impl Engine {
    /// Run one check, re-running it from step 0 when `defaults.retry`
    /// is set and the verdict is retry-eligible (spec #66; see
    /// [`check_is_retryable`]). Each attempt re-emits the check's
    /// step / assertion events; only the final attempt's failing
    /// assertions stay in `failures`.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_check_with_retry(
        &mut self,
        writer: &mut EvidenceWriter,
        run: &mut RunState,
        fixtures: &duhem_schema::FixtureCatalog,
        criterion_id: &str,
        check: &Check,
        session: &SessionResolution,
        failures: &mut Vec<CheckFailure>,
        cleanup: &mut Vec<CleanupFailure>,
    ) -> Result<(CheckVerdict, u32), EngineError> {
        let max = self.retry.map(|r| r.max).unwrap_or(0);
        let backoff = self
            .retry
            .map(|r| r.backoff)
            .unwrap_or(RetryBackoff::Exponential);
        let mut attempt = 0;
        loop {
            // Discard any failures a prior (retried) attempt left behind
            // so only the final attempt's detail reaches the reporter.
            let failures_mark = failures.len();
            let mut contexts = if check.sessions.is_some() {
                Some(CheckContexts::open_named(session, self.browser.as_ref()).await)
            } else {
                None
            };
            let attempt_result: Result<(CheckVerdict, u32), EngineError> = async {
                run.clear_fixture_outputs();

                // Check-level `setup:` (#441 Part B) runs before this
                // check's `needs:` fixtures, after criterion `setup:`.
                // Re-run every retry attempt, like fixtures.
                let mut check_setup_dispatched = false;
                let mut check_setup_abort: Option<(crate::engine::setup::AbortReason, String)> =
                    None;
                if !check.setup.is_empty() {
                    let result = crate::engine::setup::run_check_setup(
                        writer,
                        &self.registry,
                        self.browser.as_ref(),
                        run,
                        criterion_id,
                        &check.id,
                        &check.setup,
                        &self.child_process_env(writer.run_id()),
                        &mut check_setup_dispatched,
                        contexts.as_ref(),
                    )
                    .await?;
                    if let Some(reason) = result.aborted {
                        check_setup_abort = Some((
                            reason,
                            result
                                .failed_step
                                .unwrap_or_else(|| "check setup environment".to_string()),
                        ));
                    }
                }

                let mut active = Vec::new();
                let mut fixture_abort = None;
                if check_setup_abort.is_none() {
                    for name in &check.needs {
                        let fixture = &fixtures[name];
                        active.push(name.as_str());
                        let result = crate::engine::setup::run_fixture_up(
                            writer,
                            &self.registry,
                            self.browser.as_ref(),
                            run,
                            name,
                            &check.id,
                            &fixture.up,
                            &self.child_process_env(writer.run_id()),
                        )
                        .await?;
                        if let Some(reason) = result.aborted {
                            fixture_abort = Some((
                                name.clone(),
                                reason,
                                result
                                    .failed_step
                                    .unwrap_or_else(|| "fixture environment".to_string()),
                            ));
                            break;
                        }
                    }
                }
                let (cv, gated_judging_steps) = if let Some((reason, step)) = &check_setup_abort {
                    failures.push(CheckFailure {
                        criterion_id: criterion_id.to_string(),
                        check_id: check.id.clone(),
                        assertions: vec![FailedAssertion {
                            expr: "check `setup:` completed".to_string(),
                            state: VerdictState::Inconclusive(reason.cause()),
                            detail: Some(format!("check `setup:` failed at step `{step}`")),
                        }],
                        captures: Vec::new(),
                    });
                    (
                        CheckVerdict {
                            check_id: check.id.clone(),
                            state: VerdictState::Inconclusive(reason.cause()),
                        },
                        0,
                    )
                } else if let Some((name, reason, step)) = fixture_abort {
                    failures.push(CheckFailure {
                        criterion_id: criterion_id.to_string(),
                        check_id: check.id.clone(),
                        assertions: vec![FailedAssertion {
                            expr: format!("fixture `{name}` up completed"),
                            state: VerdictState::Inconclusive(reason.cause()),
                            detail: Some(format!("fixture `{name}` up failed at step `{step}`")),
                        }],
                        captures: Vec::new(),
                    });
                    (
                        CheckVerdict {
                            check_id: check.id.clone(),
                            state: VerdictState::Inconclusive(reason.cause()),
                        },
                        0,
                    )
                } else {
                    self.run_check(
                        writer,
                        run,
                        (criterion_id, check),
                        session,
                        failures,
                        contexts.as_mut(),
                    )
                    .await?
                };
                for name in active.into_iter().rev() {
                    let fixture = &fixtures[name];
                    let mut failures = crate::engine::setup::run_fixture_down(
                        writer,
                        &self.registry,
                        self.browser.as_ref(),
                        run,
                        name,
                        &check.id,
                        &fixture.down,
                        &self.child_process_env(writer.run_id()),
                    )
                    .await?;
                    for failure in &mut failures {
                        failure.step = format!("fixture `{name}`: {}", failure.step);
                    }
                    cleanup.append(&mut failures);
                }

                // Check-level `teardown:` runs after this check's fixtures
                // are torn down — including after a check `setup:` abort
                // that dispatched at least one action. Evidence-only: never
                // replaces the check's verdict. Re-run every retry attempt.
                if !check.teardown.is_empty() && check_setup_dispatched {
                    let mut teardown_failures = crate::engine::setup::run_check_teardown(
                        writer,
                        &self.registry,
                        self.browser.as_ref(),
                        run,
                        criterion_id,
                        &check.id,
                        &check.teardown,
                        &self.child_process_env(writer.run_id()),
                        contexts.as_ref(),
                    )
                    .await?;
                    for failure in &mut teardown_failures {
                        failure.step = format!("check `{}` teardown: {}", check.id, failure.step);
                    }
                    cleanup.append(&mut teardown_failures);
                }

                Ok((cv, gated_judging_steps))
            }
            .await;
            writer.set_session(None);
            if let Some(contexts) = contexts.as_mut() {
                let capture = match self.capture {
                    CapturePolicy::Off => false,
                    CapturePolicy::Always => true,
                    CapturePolicy::OnFailure => attempt_result
                        .as_ref()
                        .map_or(true, |(cv, _)| cv.state != VerdictState::Pass),
                };
                let captures = contexts.finish(writer, capture, check).await;
                if let Some(failure) = failures[failures_mark..]
                    .iter_mut()
                    .find(|f| f.check_id == check.id)
                {
                    failure.captures.extend(captures);
                }
            }
            let (cv, gated_judging_steps) = attempt_result?;
            if attempt < max && check_is_retryable(cv.state) {
                failures.truncate(failures_mark);
                attempt += 1;
                let delay = retry_delay(self.retry_backoff_base, backoff, attempt);
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                continue;
            }
            return Ok((cv, gated_judging_steps));
        }
    }

    pub(super) async fn run_check(
        &mut self,
        writer: &mut EvidenceWriter,
        run: &RunState,
        target: (&str, &Check),
        session: &SessionResolution,
        failures: &mut Vec<CheckFailure>,
        named: Option<&mut CheckContexts>,
    ) -> Result<(CheckVerdict, u32), EngineError> {
        let (criterion_id, check) = target;
        let mut ctx = RunContext::new(run);

        // A `Step.uses` not in the registry means the step can't run
        // and its outputs don't exist; per spec, the check's
        // assertions all evaluate to Inconclusive(MissingObservation).
        let any_unknown = check
            .steps
            .iter()
            .any(|s| !self.registry.contains_key(s.uses_name()));

        // A step that requires a page (production wrapper around a
        // real `Action`) but has no browser attached is an
        // environment failure: the assertions can't be exercised, so
        // the check is Inconclusive(EnvironmentError) — not
        // accidentally Pass on a literal-only assertion in the same
        // check.
        let needs_browser = check.steps.iter().any(|s| {
            self.registry
                .get(s.uses_name())
                .map(|d| d.requires_page())
                .unwrap_or(false)
        });
        let browser_missing = needs_browser && self.browser.is_none();

        // Track per-check environment failures from open_check, too:
        // a browser was attached but allocating a context failed.
        let mut environment_failed = browser_missing || session.failed;

        let is_named = named.is_some();
        let mut legacy = CheckContexts::default();
        let contexts = match named {
            Some(contexts) => contexts,
            None => &mut legacy,
        };
        environment_failed |= contexts.failed;
        if !is_named
            && !any_unknown
            && !environment_failed
            && !check.steps.is_empty()
            && let Some(browser) = self.browser.as_ref()
        {
            match session.open_check(browser).await {
                Ok(cb) => {
                    contexts.browsers.insert(None, cb);
                }
                Err(error) => {
                    debug!(%error, "open_check failed");
                    environment_failed = true;
                }
            }
        }

        // Always close contexts, including when evidence or reference resolution fails.
        let result = async {
            // Step execution loop. We emit `step_started` / `step_finished`
            // for every step the document declares, even when the step
            // can't run (unknown action, environment failure) — evidence
            // is more useful when it records what the author wrote, not
            // what the engine got around to invoking.
            let mut failed_by: Option<String> = None;
            let mut stored_error = None;
            let mut cleanup_steps = BTreeSet::new();

            // Per-step evidence (resolved `with:` + outputs) for implicit
            // judgment (#280). Empty = the step didn't run.
            let mut step_evidence = vec![StepEvidence::empty(); check.steps.len()];
            let mut gated_judging_steps = 0;
            for (idx, step) in check.steps.iter().enumerate() {
                writer.set_session(step.session.as_deref());
                let check_browser = contexts.browsers.get(&step.session);
                let storyboard = contexts
                    .storyboards
                    .entry(step.session.clone())
                    .or_default();
                let (gate, condition_error) =
                    match evaluate_gate(step, idx, failed_by.as_deref(), &ctx) {
                        Ok(gate) => (gate, None),
                        Err(error) => (None, Some(error)),
                    };
                let cleanup_step = failed_by.is_some() && gate.is_none();
                if cleanup_step {
                    cleanup_steps.insert(idx);
                }
                // Resolve template references in `with:` against whatever
                // context available without bifurcating evidence.
                let mut resolved_with = step.with.clone();
                let catalog_reference = page_reference(&step.with);
                let step_error = if condition_error.is_some() {
                    condition_error
                } else if gate.is_none() {
                    substitute_with(&mut resolved_with, &ctx).err().map(|u| {
                        EngineError::UnresolvedReference {
                            context: u.rendered_context(),
                            reference: u.reference,
                            step: step_label(step, idx),
                        }
                    })
                } else {
                    None
                };
                // Manifest `defaults.timeout` (spec #66): fill the step's
                // `timeout:` when it doesn't declare its own. A per-step
                // `timeout:` already in the payload wins; this only fills the
                // gap. With no manifest default, the action's built-in
                // `DEFAULT_TIMEOUT` (5s) remains the last resort.
                if gate.is_none()
                    && let Some(default) = self.default_timeout
                {
                    apply_default_timeout(&mut resolved_with, default);
                }

                // Collect ui/assert-element targets for the element-highlight
                // overlay (spec #214) — but only for steps that actually run.
                // A gated or otherwise unexecuted step never "looked" for
                // anything, so recording its
                // locator would be misleading evidence.
                let will_run = gate.is_none()
                    && step_error.is_none()
                    && !environment_failed
                    && self.registry.contains_key(step.uses_name());
                if will_run && let Some(t) = target_from_step(step.uses_name(), &resolved_with) {
                    contexts
                        .targets
                        .entry(step.session.clone())
                        .or_default()
                        .push(t);
                }

                let known = self.registry.contains_key(step.uses_name());
                let step_started_ms = if will_run && !matches!(self.capture, CapturePolicy::Off) {
                    match check_browser.as_ref() {
                        Some(cb) => Some(storyboard.step_started(&cb.page).await),
                        None => None,
                    }
                } else {
                    None
                };

                // Resolved here so the secret pass below reaches the same
                // action's contract without a second registry lookup. The
                // invocation itself stays *after* `StepStarted` — see the
                // registration comment below.
                let dispatcher =
                    if gate.is_some() || step_error.is_some() || !known || environment_failed {
                        None
                    } else {
                        Some(
                            self.registry
                                .get(step.uses_name())
                                .expect("known checked above"),
                        )
                    };

                crate::engine::flow::register_secrets(writer, step, &ctx);

                writer
                    .append(EventPayload::StepStarted {
                        criterion_id: criterion_id.to_string(),
                        check_id: check.id.clone(),
                        step_index: idx as u32,
                        uses: step.uses_name().to_string(),
                        // Layer comes only from the executed action's catalog (#192).
                        layer: duhem_actions::layer_for_uses(step.uses_name()).map(str::to_string),
                        with: with_to_evidence_map(&resolved_with),
                        flow: crate::engine::flow::origin(step),
                    })
                    .await?;

                if let Some(duhem_evidence::StepOutcome::Skipped {
                    reason,
                    condition,
                    operands,
                }) = gate
                {
                    if condition.is_some()
                        && self
                            .registry
                            .get(step.uses_name())
                            .is_some_and(|action| action.judges())
                    {
                        gated_judging_steps += 1;
                    }
                    writer
                        .append(EventPayload::StepFinished {
                            step_index: idx as u32,
                            outcome: duhem_evidence::StepOutcome::Skipped {
                                condition,
                                operands,
                                reason: reason.clone(),
                            },
                            detail: None,
                        })
                        .await?;
                    step_evidence[idx] = StepEvidence::skipped(reason);
                    continue;
                }

                if let Some(error) = step_error {
                    // The process-level engine error already has the pinpointed
                    // `with:` reference and enclosing expression. Preserve that
                    // exact diagnostic in evidence, through the same secret
                    // boundary as every recorded string (#494).
                    let detail = writer.mask_text(&error.to_string());
                    writer
                        .append(EventPayload::StepFinished {
                            step_index: idx as u32,
                            outcome: duhem_evidence::StepOutcome::Error,
                            detail: Some(detail),
                        })
                        .await?;
                    if !cleanup_step && stored_error.is_none() {
                        stored_error = Some(error);
                    }
                    if failed_by.is_none() {
                        failed_by = Some(display_step_label(step, idx));
                    }
                    continue;
                }

                // Invoke after `StepStarted` is persisted, then register any
                // acquired secret before anything carrying this step's
                // outputs (spec #355).
                //
                // `StepStarted` records the resolved `with:` — this step's
                // *inputs* — which cannot contain this step's own outputs,
                // so nothing leaks by persisting it first. Invoking earlier
                // to register sooner buys nothing and costs liveness:
                // `StepStarted` is what live progress folds over to put a
                // step in flight, so a slow step would go unreported for its
                // whole duration, the `… still in <uses>` heartbeat (#305)
                // could never fire, and `step_started.ts` would be stamped
                // at completion, collapsing event-derived durations.
                let dispatcher_judges = dispatcher.is_some_and(|dispatcher| dispatcher.judges());
                let execution = match dispatcher {
                    None => None,
                    Some(dispatcher) => {
                        if step.uses_name() == "ui/wait"
                            && let Err(error) = enforce_wait_ceiling(&resolved_with, self.max_wait)
                        {
                            Some(Err(error))
                        } else {
                            let page_ref: Option<&Page> = check_browser.as_ref().map(|cb| &cb.page);
                            let result = dispatcher
                                .invoke(
                                    page_ref,
                                    idx,
                                    &resolved_with,
                                    &self.child_process_env(writer.run_id()),
                                )
                                .await;
                            if let Ok(r) = &result {
                                crate::engine::secret_output::register(
                                    writer,
                                    step,
                                    idx,
                                    &dispatcher.secret_outputs(),
                                    &r.outputs,
                                )?;
                            }
                            Some(result)
                        }
                    }
                };

                let outcome = match &execution {
                    Some(Ok(r)) => {
                        // Bind raw fields + `outputs:` aliases (spec #273);
                        // see `engine::extract`.
                        if let Some(id) = step.id.as_deref() {
                            crate::engine::extract::record_step_outputs(
                                &step.outputs,
                                &r.outputs,
                                |local, v| ctx.record_output(id, local, v),
                            );
                        }
                        for (name, value) in &r.outputs {
                            writer
                                .append_observation(idx as u32, name.clone(), value.clone())
                                .await?;
                        }
                        // Retain intent + outputs so implicit judgment
                        // (#280) can speak the reason.
                        step_evidence[idx] = StepEvidence {
                            with: resolved_with.clone(),
                            outputs: r.outputs.clone(),
                            skip_reason: None,
                            catalog_reference: catalog_reference.clone(),
                            outcome: Some(r.outcome.clone()),
                            detail: None,
                        };
                        if let Outcome::Skipped { reason } = &r.outcome {
                            step_evidence[idx].skip_reason = Some(reason.clone());
                        }
                        r.outcome.clone()
                    }
                    // Step can't run (or invocation failed) — emit a
                    // synthetic Error so evidence carries "not executed"
                    // alongside the assertion cause below.
                    Some(Err(_)) | None => Outcome::Error,
                };
                // Invocation errors have no ActionResult, but their attempted
                // outcome and resolved intent still feed execution-failure
                // judgment. Successful/skipped results already populated the
                // richer record above.
                if step_evidence[idx].outcome.is_none() {
                    step_evidence[idx].with = resolved_with.clone();
                    step_evidence[idx].catalog_reference = catalog_reference;
                    step_evidence[idx].outcome = Some(outcome.clone());
                }

                let raw_detail = match (&execution, &outcome) {
                    (Some(Err(error)), _) => {
                        Some(format!("action `{}` failed: {error}", step.uses_name()))
                    }
                    (_, Outcome::Timeout) => Some(format!(
                        "action `{}` timed out{}",
                        step.uses_name(),
                        execution_deadline(&step_evidence[idx])
                    )),
                    (_, Outcome::Error) if !known => {
                        Some(format!("action `{}` is not registered", step.uses_name()))
                    }
                    (_, Outcome::Error) if environment_failed => Some(format!(
                        "action `{}` could not start because the check environment failed",
                        step.uses_name()
                    )),
                    (_, Outcome::Error) => {
                        Some(format!("action `{}` returned an error", step.uses_name()))
                    }
                    (_, Outcome::Ok | Outcome::Skipped { .. }) => None,
                };
                let detail = raw_detail.map(|detail| writer.mask_text(&detail));
                step_evidence[idx].detail = detail.clone();

                writer
                    .append(EventPayload::StepFinished {
                        step_index: idx as u32,
                        outcome: outcome_to_evidence(&outcome),
                        detail,
                    })
                    .await?;

                // Post-step storyboard frame (#337): the page remains available
                // after Ok/Error/Timeout, so capture every executed step before
                // an Error aborts later steps. The policy decision happens after
                // judgment; this is temporary bounded memory until then.
                if let (Some(started_ms), Some(cb)) = (step_started_ms, check_browser.as_ref()) {
                    storyboard
                        .capture_step(&cb.page, idx as u32, started_ms)
                        .await;
                }

                let judgment = implicit_judgment_for_step(
                    step,
                    idx,
                    dispatcher_judges,
                    self.registry.contains_key(step.uses_name()),
                    &step_evidence[idx],
                    false,
                    false,
                )
                .map(|outcome| outcome.state);
                if failed_by.is_none() && step_failed(&outcome, judgment) {
                    failed_by = Some(display_step_label(step, idx));
                }
            }

            writer.set_session(None);
            if let Some(error) = stored_error {
                return Err(error);
            }

            // Explicit `assertions:` (indices 0..len), then the implicit
            // judgment of judging steps (#253) appended after them. Both
            // paths fold into the same collections and share the
            // unknown-action / environment-failure cause prefix.
            let mut assertion_outcomes: Vec<AssertionOutcome> = Vec::new();
            // Non-passing assertions, collected for the reporter so a failing
            // run shows *which* assertion failed without trace-reading.
            let mut failed: Vec<FailedAssertion> = Vec::new();
            evaluate_explicit_assertions(
                writer,
                check,
                &ctx,
                any_unknown,
                environment_failed,
                browser_missing,
                &step_evidence,
                &cleanup_steps,
                &mut assertion_outcomes,
                &mut failed,
            )
            .await?;

            // Implicit judgment (spec #253; see `implicit_judgment_outcomes`
            // and §10.3.2): judging steps append their `satisfied == true`
            // outcomes after the explicit assertions.
            let implicit = implicit_judgment_outcomes(
                check,
                |uses| self.registry.get(uses).map(|d| d.judges()).unwrap_or(false),
                |uses| self.registry.contains_key(uses),
                &step_evidence,
                environment_failed,
                browser_missing,
                &cleanup_steps,
            );
            append_implicit_judgment(
                writer,
                &check.id,
                implicit,
                check.assertions.len(),
                &mut assertion_outcomes,
                &mut failed,
            )
            .await?;

            // Judge first, from action observations and assertions only. Replay
            // evidence is retained from the resulting state and can therefore
            // never become a judgment input.
            let outcome = CheckOutcome {
                check_id: check.id.clone(),
                assertions: assertion_outcomes,
            };
            let verdict = aggregate_check(&outcome);

            // Failure-evidence capture (spec #202): the browser is still
            // open and the failure set is known, so this is the one spot
            // where "what did the page look like" can be recorded. Rides
            // the `step_observation` blob channel under the reserved
            // `capture/` prefix — the dashboard's existing artifact
            // pipeline picks it up with no reader/SPA changes.
            let mut captures: Vec<CapturedArtifact> = Vec::new();
            let wants_capture = match self.capture {
                CapturePolicy::Off => false,
                CapturePolicy::Always => true,
                CapturePolicy::OnFailure => !matches!(verdict.state, VerdictState::Pass),
            };
            if !is_named {
                captures = contexts.finish(writer, wants_capture, check).await;
            }
            writer.set_session(None);

            // Surface this check's failing assertions only when the check
            // itself didn't pass (a check can pass with some inconclusive
            // assertions aggregated away; we don't cry wolf on those).
            if !matches!(verdict.state, VerdictState::Pass) && !failed.is_empty() {
                failures.push(CheckFailure {
                    criterion_id: criterion_id.to_string(),
                    check_id: check.id.clone(),
                    assertions: failed,
                    captures,
                });
            }
            Ok((verdict, gated_judging_steps))
        }
        .await;
        writer.set_session(None);
        if !is_named {
            contexts.finish(writer, false, check).await;
        }
        result
    }
}
