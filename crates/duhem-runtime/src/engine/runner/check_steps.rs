//! Shared action execution for ordinary check steps and loop iterations.
use super::*;

impl Engine {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_check_steps(
        &self,
        writer: &mut EvidenceWriter,
        criterion_id: &str,
        check: &Check,
        ctx: &mut RunContext<'_>,
        contexts: &mut CheckContexts,
        indices: std::ops::Range<usize>,
        iteration: Option<u32>,
        environment_failed: bool,
        failed_by: &mut Option<String>,
        stored_error: &mut Option<EngineError>,
        cleanup_steps: &mut BTreeSet<usize>,
        step_evidence: &mut [StepEvidence],
        gated_judging_steps: &mut u32,
    ) -> Result<(), EngineError> {
        for idx in indices {
            let step = &check.steps[idx];
            writer.set_session(step.session.as_deref());
            let check_browser = contexts.browsers.get(&step.session);
            let storyboard = contexts
                .storyboards
                .entry(step.session.clone())
                .or_default();
            let (gate, condition_error) = match evaluate_gate(step, idx, failed_by.as_deref(), ctx)
            {
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
                substitute_with(&mut resolved_with, ctx).err().map(|u| {
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

            crate::engine::flow::register_secrets(writer, step, ctx);

            writer
                .append(EventPayload::StepStarted {
                    criterion_id: criterion_id.to_string(),
                    check_id: check.id.clone(),
                    step_index: idx as u32,
                    uses: step.uses_name().to_string(),
                    // Layer comes only from the executed action's catalog (#192).
                    layer: duhem_actions::layer_for_uses(step.uses_name()).map(str::to_string),
                    with: with_to_evidence_map(&resolved_with),
                    flow: {
                        let mut flow = crate::engine::flow::origin(step);
                        if let Some(flow) = &mut flow {
                            flow.iteration = iteration;
                        }
                        flow
                    },
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
                    *gated_judging_steps += 1;
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
                    *stored_error = Some(error);
                }
                if failed_by.is_none() {
                    *failed_by = Some(display_step_label(step, idx));
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
                *failed_by = Some(display_step_label(step, idx));
            }
        }

        Ok(())
    }
}
