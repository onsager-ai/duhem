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
        let actions = || {
            check.steps.iter().flat_map(|s| {
                if s.for_each.is_some() {
                    s.for_each_body.iter()
                } else {
                    std::slice::from_ref(s).iter()
                }
            })
        };

        // A `Step.uses` not in the registry means the step can't run
        // and its outputs don't exist; per spec, the check's
        // assertions all evaluate to Inconclusive(MissingObservation).
        let any_unknown = actions().any(|s| !self.registry.contains_key(s.uses_name()));

        // A step that requires a page (production wrapper around a
        // real `Action`) but has no browser attached is an
        // environment failure: the assertions can't be exercised, so
        // the check is Inconclusive(EnvironmentError) — not
        // accidentally Pass on a literal-only assertion in the same
        // check.
        let needs_browser = actions().any(|s| {
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
            let (assertion_outcomes, failed, gated_judging_steps) = self
                .run_check_iterations(
                    writer,
                    criterion_id,
                    check,
                    &mut ctx,
                    contexts,
                    environment_failed,
                    browser_missing,
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
