//! Per-step dispatch and `for_each:` iteration (spec #443 Tier 1).
//!
//! Split out of `setup.rs` (which was over the per-file token budget)
//! rather than exempted: this is genuinely one cohesive unit — one
//! step's gate/resolve/dispatch/evidence contract
//! ([`process_lifecycle_step`]) plus the loop that reuses it once per
//! `for_each:` element ([`run_for_each_step`]) — that setup.rs's
//! lifecycle-block orchestration (`run_lifecycle_steps` and its
//! `run_setup`/`run_teardown`/fixture/criterion/check wrappers) calls
//! into but doesn't need the internals of.

use std::collections::BTreeMap;

use duhem_actions::Page;
use duhem_actions::{CheckBrowser, Outcome};
use duhem_evidence::{EventPayload, EvidenceWriter, StepOutcome, StepPhase};
use duhem_schema::Step;

use crate::engine::context::RunState;
use crate::engine::gating::{evaluate as evaluate_gate, step_failed};
use crate::engine::registry::{ActionRegistry, Dispatch};
use crate::engine::runner::{
    CleanupFailure, EngineError, StepEvidence, display_step_label, implicit_judgment_for_step,
    step_label,
};
use crate::engine::setup::{AbortReason, HookScope};
use crate::engine::template::substitute_with;
use crate::engine::translate::{outcome_to_evidence, with_to_evidence_map};

/// The context for resolving one `for_each:` iteration's body step:
/// the `as:` binding name (`None` when the loop declared no `as:`),
/// the current element, and the `0`-based iteration ordinal for
/// evidence provenance (#443 Tier 1).
pub(crate) struct LoopIterationCtx<'a> {
    pub(crate) as_name: Option<&'a str>,
    pub(crate) element: &'a crate::eval::Value,
    pub(crate) iteration: u32,
}

/// Build the `RunContext` a lifecycle step resolves its `if:`/`with:`
/// against, binding `$<as>` to the current element when `loop_ctx` is
/// `Some` and the loop declared an `as:` name (#443 Tier 1).
fn loop_run_context<'r>(
    run: &'r RunState,
    loop_ctx: Option<&LoopIterationCtx<'_>>,
) -> crate::engine::context::RunContext<'r> {
    let ctx = crate::engine::context::RunContext::new(run);
    match loop_ctx.and_then(|l| l.as_name.map(|name| (name, l.element))) {
        Some((name, element)) => ctx.with_loop_binding(name.to_string(), element.clone()),
        None => ctx,
    }
}

/// Dispatch one ordinary lifecycle action step: gate, resolve `with:`,
/// invoke, and record evidence + abort/cleanup bookkeeping. Shared by
/// the plain per-step path and by `for_each:`'s per-iteration body
/// steps (#443 Tier 1) — `loop_ctx` is `Some` only for the latter, and
/// makes `$<as>` resolve against the current element while tagging
/// this step's evidence with its iteration.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_lifecycle_step(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    setup_browser: Option<&CheckBrowser>,
    run: &mut RunState,
    child_env: &BTreeMap<String, String>,
    phase: StepPhase,
    scope: HookScope<'_>,
    environment_failed: bool,
    step: &Step,
    idx: usize,
    loop_ctx: Option<&LoopIterationCtx<'_>>,
    contexts: Option<&super::session::CheckContexts>,
    dispatched: &mut bool,
    aborted: &mut Option<AbortReason>,
    failed_by: &mut Option<String>,
    stored_error: &mut Option<EngineError>,
    cleanup: &mut Vec<CleanupFailure>,
) -> Result<(), EngineError> {
    let (fixture_name, check_id, criterion_id) = scope.evidence_fields();
    writer.set_session(step.session.as_deref());

    // The outcome gate runs first — including for value expressions,
    // which carry `success` semantics (see `gating::skip_reason`).
    // Only once it passes is the expression itself evaluated, so a
    // step blocked by an earlier failure is gated cleanly rather
    // than failing on operands that failure left unresolvable.
    // `gating::evaluate` — not a second, hand-rolled evaluation — is
    // what lets a loop-body step gated by a value condition record
    // `condition`/`operands` the same way any other gated step does
    // (#511's shrunken-claim-set reporting depends on this).
    let ctx = loop_run_context(run, loop_ctx);
    let (gate, condition_error) = match evaluate_gate(step, idx, failed_by.as_deref(), &ctx) {
        Ok(gate) => (gate, None),
        Err(error) => (None, Some(error)),
    };
    let cleanup_step = phase == StepPhase::Teardown || (failed_by.is_some() && gate.is_none());
    // Setup steps see the run state (inputs, env, uuid, plus any
    // outputs already published by earlier setup steps in this
    // same block, and — inside a `for_each:` body — the current
    // iteration's `as:` binding). The view is read-only against the
    // run state — we feed it through a `RunContext` to reuse the
    // existing template substitution.
    let mut resolved_with = step.with.clone();
    let step_error = if condition_error.is_some() {
        condition_error
    } else if gate.is_none() {
        substitute_with(&mut resolved_with, &ctx)
            .err()
            .map(|u| EngineError::UnresolvedReference {
                context: u.rendered_context(),
                reference: u.reference,
                step: step_label(step, idx),
            })
    } else {
        None
    };

    let iteration = loop_ctx.map(|l| l.iteration);
    append_setup_started(writer, phase, step, idx, &resolved_with, scope, iteration).await?;
    if let Some(StepOutcome::Skipped {
        reason,
        condition,
        operands,
    }) = gate
    {
        writer
            .append(EventPayload::SetupStepFinished {
                phase,
                step_index: idx as u32,
                outcome: StepOutcome::Skipped {
                    reason,
                    condition,
                    operands,
                },
                detail: None,
                fixture_name: fixture_name.clone(),
                check_id: check_id.clone(),
                criterion_id: criterion_id.clone(),
            })
            .await?;
        return Ok(());
    }

    let mut error_detail = step_error.as_ref().map(ToString::to_string);
    let (outcome, failed) = if step_error.is_some() || environment_failed {
        if error_detail.is_none() {
            error_detail = Some(format!(
                "action `{}` could not start because the setup environment failed",
                step.uses_name()
            ));
        }
        (Outcome::Error, true)
    } else {
        match registry.get(step.uses_name()) {
            None => {
                error_detail = Some(format!("action `{}` is not registered", step.uses_name()));
                (Outcome::Error, true)
            }
            Some(dispatcher) => {
                *dispatched = true;
                let page_ref: Option<&Page> = match contexts {
                    Some(contexts) => contexts.browsers.get(&step.session).map(|cb| &cb.page),
                    None => setup_browser.map(|cb| &cb.page),
                };
                match invoke_and_record(
                    dispatcher.as_ref(),
                    page_ref,
                    phase,
                    idx,
                    &resolved_with,
                    SetupInvocation {
                        step,
                        run,
                        writer,
                        child_env,
                        scope,
                    },
                )
                .await
                {
                    Ok((outcome, failed, action_detail)) => {
                        error_detail = action_detail;
                        (outcome, failed)
                    }
                    Err(error) => {
                        error_detail = Some(error.to_string());
                        if !cleanup_step && stored_error.is_none() {
                            *stored_error = Some(error);
                        }
                        (Outcome::Error, true)
                    }
                }
            }
        }
    };

    if let Some(error) = step_error
        && !cleanup_step
        && stored_error.is_none()
    {
        *stored_error = Some(error);
    }

    let evidence_outcome = outcome_to_evidence(&outcome);
    let detail = match &outcome {
        Outcome::Timeout => {
            error_detail.or_else(|| Some(format!("action `{}` timed out", step.uses_name())))
        }
        Outcome::Error => error_detail
            .or_else(|| Some(format!("action `{}` returned an error", step.uses_name()))),
        Outcome::Ok | Outcome::Skipped { .. } => None,
    }
    .map(|detail| writer.mask_text(&detail));
    writer
        .append(EventPayload::SetupStepFinished {
            phase,
            step_index: idx as u32,
            outcome: evidence_outcome.clone(),
            detail: detail.clone(),
            fixture_name: fixture_name.clone(),
            check_id: check_id.clone(),
            criterion_id: criterion_id.clone(),
        })
        .await?;

    if phase == StepPhase::Teardown && failed {
        let detail = detail.or_else(|| {
            matches!(outcome, Outcome::Ok).then(|| "judging action reported failure".to_string())
        });
        cleanup.push(CleanupFailure {
            step: display_step_label(step, idx),
            outcome: evidence_outcome,
            detail,
        });
    }

    if aborted.is_none() {
        *aborted = match outcome {
            Outcome::Timeout => Some(AbortReason::Timeout),
            Outcome::Error => Some(AbortReason::ActionError),
            Outcome::Ok if failed => Some(AbortReason::Environment),
            Outcome::Ok | Outcome::Skipped { .. } => None,
        };
        if aborted.is_some() {
            *failed_by = Some(display_step_label(step, idx));
        }
    }
    Ok(())
}

/// Run one `for_each:` step (#443 Tier 1): gate the loop as a whole,
/// read its source array once (R2), enforce `max:` as a hard ceiling
/// (R1 — exceeding it is a failure, never a silent truncation), and
/// then dispatch the body once per element through
/// [`process_lifecycle_step`] — the same per-step machinery every
/// other lifecycle action uses, so each iteration's steps are real
/// steps with real evidence (R4) and the existing `failed_by`/
/// `if: always`/`if: failure` gating applies unchanged across
/// iterations. An empty array is zero iterations, not an error (R7).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_for_each_step(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    setup_browser: Option<&CheckBrowser>,
    run: &mut RunState,
    child_env: &BTreeMap<String, String>,
    phase: StepPhase,
    scope: HookScope<'_>,
    environment_failed: bool,
    step: &Step,
    idx: usize,
    contexts: Option<&super::session::CheckContexts>,
    dispatched: &mut bool,
    aborted: &mut Option<AbortReason>,
    failed_by: &mut Option<String>,
    stored_error: &mut Option<EngineError>,
    cleanup: &mut Vec<CleanupFailure>,
) -> Result<(), EngineError> {
    let (fixture_name, check_id, criterion_id) = scope.evidence_fields();
    let expr = step
        .for_each
        .as_ref()
        .expect("caller checked step.for_each.is_some()");
    // A human-readable stand-in for `uses:` in this step's evidence —
    // a `for_each:` wrapper has no action of its own to name. Uses
    // whichever body form was authored so the trace still says what
    // the loop does.
    let uses_label = step
        .uses
        .clone()
        .or_else(|| step.call.clone())
        .unwrap_or_else(|| "for_each".to_string());

    // The loop's own outcome gate (its `if:`) is resolved once, in the
    // outer scope — never against a loop-bound element, since the
    // array hasn't been read yet at this point. Same `gating::evaluate`
    // every other step gate goes through (see `process_lifecycle_step`),
    // not a second hand-rolled evaluation.
    let gate_ctx = crate::engine::context::RunContext::new(run);
    let (gate, condition_error) = match evaluate_gate(step, idx, failed_by.as_deref(), &gate_ctx) {
        Ok(gate) => (gate, None),
        Err(error) => (None, Some(error)),
    };
    let cleanup_step = phase == StepPhase::Teardown || failed_by.is_some();

    writer
        .append(EventPayload::SetupStepStarted {
            phase,
            step_index: idx as u32,
            uses: uses_label,
            layer: None,
            with: BTreeMap::new(),
            fixture_name: fixture_name.clone(),
            check_id: check_id.clone(),
            criterion_id: criterion_id.clone(),
            flow: None,
        })
        .await?;

    if let Some(outcome) = gate {
        writer
            .append(EventPayload::SetupStepFinished {
                phase,
                step_index: idx as u32,
                outcome,
                detail: None,
                fixture_name: fixture_name.clone(),
                check_id: check_id.clone(),
                criterion_id: criterion_id.clone(),
            })
            .await?;
        return Ok(());
    }

    if let Some(error) = condition_error {
        let detail = writer.mask_text(&error.to_string());
        writer
            .append(EventPayload::SetupStepFinished {
                phase,
                step_index: idx as u32,
                outcome: StepOutcome::Error,
                detail: Some(detail),
                fixture_name: fixture_name.clone(),
                check_id: check_id.clone(),
                criterion_id: criterion_id.clone(),
            })
            .await?;
        if aborted.is_none() {
            *aborted = Some(AbortReason::ActionError);
            *failed_by = Some(display_step_label(step, idx));
        }
        if !cleanup_step && stored_error.is_none() {
            *stored_error = Some(error);
        }
        return Ok(());
    }

    // R2: the array is read exactly once, right here.
    let ctx = crate::engine::context::RunContext::new(run);
    let items = match crate::eval::eval_to_value(&expr.parsed, &ctx) {
        Ok(crate::eval::Value::Array(items)) => items,
        Ok(other) => {
            let detail = writer.mask_text(&format!(
                "for_each `{}` must evaluate to an array, got {:?}",
                expr.raw,
                other.shape()
            ));
            writer
                .append(EventPayload::SetupStepFinished {
                    phase,
                    step_index: idx as u32,
                    outcome: StepOutcome::Error,
                    detail: Some(detail),
                    fixture_name: fixture_name.clone(),
                    check_id: check_id.clone(),
                    criterion_id: criterion_id.clone(),
                })
                .await?;
            if aborted.is_none() {
                *aborted = Some(AbortReason::ActionError);
                *failed_by = Some(display_step_label(step, idx));
            }
            return Ok(());
        }
        Err(cause) => {
            let error = EngineError::UnresolvedReference {
                reference: expr.raw.clone(),
                context: format!(" (for_each source could not be evaluated: {cause:?})"),
                step: step_label(step, idx),
            };
            let detail = writer.mask_text(&error.to_string());
            writer
                .append(EventPayload::SetupStepFinished {
                    phase,
                    step_index: idx as u32,
                    outcome: StepOutcome::Error,
                    detail: Some(detail),
                    fixture_name: fixture_name.clone(),
                    check_id: check_id.clone(),
                    criterion_id: criterion_id.clone(),
                })
                .await?;
            if aborted.is_none() {
                *aborted = Some(AbortReason::ActionError);
                *failed_by = Some(display_step_label(step, idx));
            }
            if !cleanup_step && stored_error.is_none() {
                *stored_error = Some(error);
            }
            return Ok(());
        }
    };

    // R1: `max:` is a hard ceiling. Exceeding it is a failure that
    // names the actual count and the ceiling — never a silent
    // truncation to the first `max` elements, which would report
    // having done less than the Verification Definition budgeted for.
    // Validation requires `max:`; a missing value here (an
    // already-invalid definition reaching the runtime some other way)
    // is treated as the tightest possible ceiling rather than
    // panicking or silently allowing everything through.
    let max = step.max.unwrap_or(0);
    if items.len() as u64 > u64::from(max) {
        let detail = writer.mask_text(&format!(
            "for_each `{}` produced {} item(s), exceeding max: {max}",
            expr.raw,
            items.len()
        ));
        writer
            .append(EventPayload::SetupStepFinished {
                phase,
                step_index: idx as u32,
                outcome: StepOutcome::Error,
                detail: Some(detail),
                fixture_name: fixture_name.clone(),
                check_id: check_id.clone(),
                criterion_id: criterion_id.clone(),
            })
            .await?;
        if aborted.is_none() {
            *aborted = Some(AbortReason::ActionError);
            *failed_by = Some(display_step_label(step, idx));
        }
        return Ok(());
    }

    // R7: an empty array is zero iterations, not an error.
    writer
        .append(EventPayload::SetupStepFinished {
            phase,
            step_index: idx as u32,
            outcome: StepOutcome::Ok,
            detail: None,
            fixture_name: fixture_name.clone(),
            check_id: check_id.clone(),
            criterion_id: criterion_id.clone(),
        })
        .await?;

    let as_name = step.as_binding.as_deref();
    for (iteration, element) in items.iter().enumerate() {
        let loop_ctx = LoopIterationCtx {
            as_name,
            element,
            iteration: iteration as u32,
        };
        for body_step in &step.for_each_body {
            process_lifecycle_step(
                writer,
                registry,
                setup_browser,
                run,
                child_env,
                phase,
                scope,
                environment_failed,
                body_step,
                idx,
                Some(&loop_ctx),
                contexts,
                dispatched,
                aborted,
                failed_by,
                stored_error,
                cleanup,
            )
            .await?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn append_setup_started(
    writer: &mut EvidenceWriter,
    phase: StepPhase,
    step: &Step,
    idx: usize,
    resolved_with: &serde_yml::Value,
    scope: HookScope<'_>,
    iteration: Option<u32>,
) -> Result<(), EngineError> {
    let (fixture_name, check_id, criterion_id) = scope.evidence_fields();
    let mut flow = crate::engine::flow::origin(step);
    if let (Some(flow), Some(iteration)) = (flow.as_mut(), iteration) {
        // Patch in the current `for_each:` iteration (#443 Tier 1) —
        // the schema-time template (`Step::for_each_body`) has no
        // iteration yet, since it's cloned once per run and reused
        // for every element.
        flow.iteration = Some(iteration);
    }
    writer
        .append(EventPayload::SetupStepStarted {
            phase,
            step_index: idx as u32,
            uses: step.uses_name().to_string(),
            // Same honesty contract as the per-check tag (#192).
            layer: duhem_actions::layer_for_uses(step.uses_name()).map(str::to_string),
            with: with_to_evidence_map(resolved_with),
            fixture_name,
            check_id,
            criterion_id,
            flow,
        })
        .await?;
    Ok(())
}

/// Invoke one setup-step dispatcher, write a `SetupStepObservation`
/// for every output, and publish scalar outputs onto
/// `RunState.setup_outputs` so checks can reference them as
/// `$setup.<id>.outputs.<name>`.
struct SetupInvocation<'a> {
    step: &'a Step,
    run: &'a mut RunState,
    writer: &'a mut EvidenceWriter,
    child_env: &'a BTreeMap<String, String>,
    scope: HookScope<'a>,
}

async fn invoke_and_record(
    dispatcher: &dyn Dispatch,
    page: Option<&Page>,
    phase: StepPhase,
    idx: usize,
    resolved_with: &serde_yml::Value,
    invocation: SetupInvocation<'_>,
) -> Result<(Outcome, bool, Option<String>), EngineError> {
    let SetupInvocation {
        step,
        run,
        writer,
        child_env,
        scope,
    } = invocation;
    // The caller persisted `SetupStepStarted` before dispatch so slow
    // actions and gated skips share one honest lifecycle shape.
    let result = dispatcher.invoke(page, idx, resolved_with, child_env).await;
    let outcome = match &result {
        Ok(r) => r.outcome.clone(),
        Err(_) => Outcome::Error,
    };
    let action_detail = result
        .as_ref()
        .err()
        .map(|error| format!("action `{}` failed: {error}", step.uses_name()));
    if let Ok(r) = &result {
        crate::engine::secret_output::register(
            writer,
            step,
            idx,
            &dispatcher.secret_outputs(),
            &r.outputs,
        )?;
    }
    if let Ok(r) = &result {
        // Bind raw fields (native names) + `outputs:` aliases (spec
        // #273) as `$setup.<id>.outputs.<name>`. Symmetric with the
        // per-check path in `runner.rs`; see `engine::extract`.
        if let Some(id) = step.id.as_deref() {
            crate::engine::extract::record_step_outputs(&step.outputs, &r.outputs, |local, v| {
                if let HookScope::Fixture(fixture, _) = scope {
                    run.record_fixture_output(fixture, id, local, v);
                } else {
                    // Leaf, criterion, and check `setup:`/`teardown:`
                    // steps all publish into the same `$setup.<id>`
                    // namespace (#441 Part B): execution is strictly
                    // sequential and validation forbids an inner
                    // scope's step id from shadowing an outer scope's
                    // still-open id, so a flat map is safe and lets
                    // an outer teardown read its own outer setup's
                    // outputs with no new reference syntax.
                    run.record_setup_output(id, local, v);
                }
            });
        }
        for (name, value) in &r.outputs {
            // Setup observations get their own event variant so
            // readers can attribute the observation to the
            // run-level setup block, not a per-check step.
            append_setup_observation(
                writer,
                phase,
                idx as u32,
                name.clone(),
                value.clone(),
                scope,
            )
            .await?;
        }
    }
    let outputs = result
        .as_ref()
        .map(|action| &action.outputs)
        .ok()
        .cloned()
        .unwrap_or_default();
    let evidence = StepEvidence {
        with: resolved_with.clone(),
        outputs,
        skip_reason: match &outcome {
            Outcome::Skipped { reason } => Some(reason.clone()),
            _ => None,
        },
        catalog_reference: None,
        outcome: Some(outcome.clone()),
        detail: action_detail
            .as_deref()
            .map(|detail| writer.mask_text(detail)),
    };
    let judgment = (!scope.is_fixture())
        .then(|| {
            implicit_judgment_for_step(
                step,
                idx,
                dispatcher.judges(),
                true,
                &evidence,
                false,
                false,
            )
        })
        .flatten()
        .map(|outcome| outcome.state);
    let failed = step_failed(&outcome, judgment);
    Ok((outcome, failed, action_detail))
}

/// Mirror of `EvidenceWriter::append_observation` for setup. The
/// inline-vs-blob policy (`BLOB_INLINE_THRESHOLD_BYTES`) is shared;
/// only the event variant differs.
async fn append_setup_observation(
    writer: &mut EvidenceWriter,
    phase: StepPhase,
    step_index: u32,
    output_name: String,
    value: serde_json::Value,
    scope: HookScope<'_>,
) -> Result<(), EngineError> {
    use duhem_evidence::{BLOB_INLINE_THRESHOLD_BYTES, ObservationValue};
    let (fixture_name, check_id, criterion_id) = scope.evidence_fields();
    let inline_bytes = serde_json::to_vec(&value).map_err(duhem_evidence::WriterError::from)?;
    let obs = if inline_bytes.len() > BLOB_INLINE_THRESHOLD_BYTES {
        let sha = writer.write_blob(&inline_bytes).await?;
        ObservationValue::Blob {
            blob_sha256: sha.0,
            mask_counts: BTreeMap::new(),
        }
    } else {
        ObservationValue::Inline { value }
    };
    writer
        .append(EventPayload::SetupStepObservation {
            phase,
            step_index,
            output_name,
            value: obs,
            fixture_name,
            check_id,
            criterion_id,
        })
        .await?;
    Ok(())
}
