//! Run-level `setup:` execution.
//!
//! Per the spec on issue #20: setup runs once per run, before any
//! criterion, against its own browser context. Step outputs are
//! published into `RunState.setup_outputs` so checks can reference
//! them as `$setup.<id>.outputs.<name>`; browser state does *not*
//! cross the boundary — each check still opens its own browser per
//! issue #15.
//!
//! Failure policy is three-state-faithful (`docs/duhem-spec.md` §7.6):
//! `Outcome::Error` or `Outcome::Timeout` from any setup step aborts
//! setup, no criterion runs, and the run verdict is `Inconclusive` —
//! "we couldn't observe the workload in the state the Verification
//! Definition claims to verify". The specific
//! `InconclusiveCause` preserves the abort trigger: a setup-step
//! `Timeout` surfaces as `Inconclusive(Timeout)`, while an `Error`,
//! an unknown-action step, or a missing browser surfaces as
//! `Inconclusive(EnvironmentError)` — the same cause family the
//! per-check path uses for analogous infrastructure failures.

use std::collections::BTreeMap;

use duhem_actions::Page;
use duhem_actions::{Outcome, RunBrowser};
use duhem_evidence::{EventPayload, EvidenceWriter, StepOutcome, StepPhase};
use duhem_judge::InconclusiveCause;
use duhem_schema::{Step, StepCondition};
use tracing::debug;

use crate::engine::context::RunState;
use crate::engine::gating::{skip_reason as gate_skip_reason, step_failed};
use crate::engine::registry::{ActionRegistry, Dispatch};
use crate::engine::runner::{
    CleanupFailure, EngineError, StepEvidence, display_step_label, implicit_judgment_for_step,
    step_label,
};
use crate::engine::template::substitute_with;
use crate::engine::translate::{outcome_to_evidence, with_to_evidence_map};

/// Why a setup block aborted. Distinct from a generic `aborted: bool`
/// so the engine can map the trigger to the right
/// `InconclusiveCause` — a setup-step `Timeout` and a missing-browser
/// `EnvironmentError` are both Inconclusive, but conflating them
/// would lose useful telemetry on the trace and the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbortReason {
    /// A setup step returned `Outcome::Timeout` — the action ran but
    /// didn't reach its requested state within `timeout:`.
    Timeout,
    /// A setup step returned `Outcome::Error`, used an unknown
    /// `Step.uses`, or the runtime couldn't provision a setup browser
    /// when one was required.
    Environment,
}

impl AbortReason {
    /// Map the abort trigger to a judge-level `InconclusiveCause` so
    /// `Engine::run` can short-circuit to a meaningful `RunVerdict`.
    pub fn cause(self) -> InconclusiveCause {
        match self {
            AbortReason::Timeout => InconclusiveCause::Timeout,
            AbortReason::Environment => InconclusiveCause::EnvironmentError,
        }
    }
}

/// Outcome of walking the run-level `setup:` block.
#[derive(Debug)]
pub(crate) struct SetupResult {
    /// `Some(reason)` when any step produced `Outcome::Error` or
    /// `Outcome::Timeout` (or an environmental precondition failed)
    /// and the rest of setup was skipped. Drives the engine's
    /// "skip criteria, emit Inconclusive" path.
    pub aborted: Option<AbortReason>,
    pub failed_step: Option<String>,
}

struct LifecycleResult {
    aborted: Option<AbortReason>,
    failed_step: Option<String>,
    cleanup: Vec<CleanupFailure>,
}

/// Execute every step in `setup` once, emitting `Setup*` evidence
/// events and recording any outputs onto `run.setup_outputs`.
/// Caller is responsible for skipping the call entirely when
/// `setup.is_empty()` so the wire shape stays byte-identical for
/// setup-free definitions.
#[cfg(test)]
pub(crate) async fn run_setup(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    setup: &[Step],
    child_env: &BTreeMap<String, String>,
) -> Result<SetupResult, EngineError> {
    let mut dispatched = false;
    run_setup_tracking(
        writer,
        registry,
        browser,
        run,
        setup,
        child_env,
        &mut dispatched,
    )
    .await
}

pub(crate) async fn run_setup_tracking(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    setup: &[Step],
    child_env: &BTreeMap<String, String>,
    dispatched: &mut bool,
) -> Result<SetupResult, EngineError> {
    let result = run_lifecycle_steps(
        writer,
        registry,
        browser,
        run,
        setup,
        child_env,
        StepPhase::Setup,
        dispatched,
        None,
    )
    .await?;
    Ok(SetupResult {
        aborted: result.aborted,
        failed_step: result.failed_step,
    })
}

/// Drain leaf cleanup without allowing action failures or step-local
/// engine errors to replace the run's verdict or an earlier error.
pub(crate) async fn run_teardown(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    teardown: &[Step],
    child_env: &BTreeMap<String, String>,
) -> Result<Vec<CleanupFailure>, EngineError> {
    let mut dispatched = false;
    Ok(run_lifecycle_steps(
        writer,
        registry,
        browser,
        run,
        teardown,
        child_env,
        StepPhase::Teardown,
        &mut dispatched,
        None,
    )
    .await?
    .cleanup)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_fixture_up(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    fixture: &str,
    check_id: &str,
    steps: &[Step],
    child_env: &BTreeMap<String, String>,
) -> Result<SetupResult, EngineError> {
    let mut dispatched = false;
    let result = run_lifecycle_steps(
        writer,
        registry,
        browser,
        run,
        steps,
        child_env,
        StepPhase::Setup,
        &mut dispatched,
        Some((fixture, check_id)),
    )
    .await?;
    Ok(SetupResult {
        aborted: result.aborted,
        failed_step: result.failed_step,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_fixture_down(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    fixture: &str,
    check_id: &str,
    steps: &[Step],
    child_env: &BTreeMap<String, String>,
) -> Result<Vec<CleanupFailure>, EngineError> {
    let mut dispatched = false;
    Ok(run_lifecycle_steps(
        writer,
        registry,
        browser,
        run,
        steps,
        child_env,
        StepPhase::Teardown,
        &mut dispatched,
        Some((fixture, check_id)),
    )
    .await?
    .cleanup)
}

#[allow(clippy::too_many_arguments)]
async fn run_lifecycle_steps(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    steps: &[Step],
    child_env: &BTreeMap<String, String>,
    phase: StepPhase,
    dispatched: &mut bool,
    fixture_scope: Option<(&str, &str)>,
) -> Result<LifecycleResult, EngineError> {
    writer
        .append(EventPayload::SetupStarted {
            phase,
            step_count: steps.len() as u32,
            fixture_name: fixture_scope.map(|scope| scope.0.to_string()),
            check_id: fixture_scope.map(|scope| scope.1.to_string()),
        })
        .await?;

    // Decide up front whether any step in this block needs a real
    // page. Mirrors the per-check logic in `Engine::run_check` so
    // setup behaves the same way on an env-failure path.
    let needs_browser = steps.iter().any(|s| {
        registry
            .get(s.uses_name())
            .map(|d| d.requires_page())
            .unwrap_or(false)
    });
    let any_unknown = steps.iter().any(|s| !registry.contains_key(s.uses_name()));
    let browser_missing = needs_browser && browser.is_none();
    let mut environment_failed = browser_missing || any_unknown;

    // Setup gets its own browser context, never shared with checks.
    let mut setup_browser = None;
    if !environment_failed
        && !steps.is_empty()
        && let Some(b) = browser
    {
        match b.open_check().await {
            Ok(cb) => setup_browser = Some(cb),
            Err(e) => {
                debug!(error = %e, ?phase, "open_check for lifecycle steps failed");
                environment_failed = true;
            }
        }
    }

    // First-cause-wins: once we record an abort reason, later steps
    // are short-circuited as `Error` for evidence but the verdict
    // cause stays pinned to the original trigger. Matches the
    // judge's "first inconclusive cause wins" fold (#16 §7.6).
    let mut aborted: Option<AbortReason> = if environment_failed {
        Some(AbortReason::Environment)
    } else {
        None
    };
    let mut failed_by = environment_failed.then(|| "setup environment".to_string());
    let mut stored_error = None;
    let mut cleanup = Vec::new();
    for (idx, step) in steps.iter().enumerate() {
        // The outcome gate runs first — including for value expressions,
        // which carry `success` semantics (see `gating::skip_reason`).
        // Only once it passes is the expression itself evaluated, so a
        // step blocked by an earlier failure is gated cleanly rather
        // than failing on operands that failure left unresolvable.
        let mut condition_error = None;
        let gate_reason = match gate_skip_reason(&step.condition, failed_by.as_deref()) {
            Some(blocked) => Some(blocked),
            None => match &step.condition {
                StepCondition::Expr(expr) => {
                    let ctx = crate::engine::context::RunContext::new(run);
                    match crate::eval(&expr.parsed, &ctx) {
                        crate::EvalResult::True => None,
                        crate::EvalResult::False => {
                            Some(format!("condition `{}` evaluated false", expr.raw))
                        }
                        crate::EvalResult::Inconclusive(cause) => {
                            condition_error = Some(EngineError::UnresolvedReference {
                                reference: expr.raw.clone(),
                                context: format!(" (condition could not be evaluated: {cause:?})"),
                                step: step_label(step, idx),
                            });
                            None
                        }
                    }
                }
                _ => None,
            },
        };
        let cleanup_step =
            phase == StepPhase::Teardown || (failed_by.is_some() && gate_reason.is_none());
        // Setup steps see the run state (inputs, env, uuid, plus any
        // outputs already published by earlier setup steps in this
        // same block). The view is read-only against the run state —
        // we feed it through a `RunContext` to reuse the existing
        // template substitution.
        let ctx = crate::engine::context::RunContext::new(run);
        let mut resolved_with = step.with.clone();
        let step_error = if condition_error.is_some() {
            condition_error
        } else if gate_reason.is_none() {
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

        append_setup_started(writer, phase, step, idx, &resolved_with, fixture_scope).await?;
        if let Some(reason) = gate_reason {
            writer
                .append(EventPayload::SetupStepFinished {
                    phase,
                    step_index: idx as u32,
                    outcome: StepOutcome::Skipped { reason },
                    fixture_name: fixture_scope.map(|scope| scope.0.to_string()),
                    check_id: fixture_scope.map(|scope| scope.1.to_string()),
                })
                .await?;
            continue;
        }

        let mut error_detail = step_error.as_ref().map(ToString::to_string);
        let (outcome, failed) = if step_error.is_some() || environment_failed {
            (Outcome::Error, true)
        } else {
            match registry.get(step.uses_name()) {
                None => (Outcome::Error, true),
                Some(dispatcher) => {
                    *dispatched = true;
                    let page_ref: Option<&Page> = setup_browser.as_ref().map(|cb| &cb.page);
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
                            fixture_scope,
                        },
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(error) => {
                            error_detail = Some(error.to_string());
                            if !cleanup_step && stored_error.is_none() {
                                stored_error = Some(error);
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
            stored_error = Some(error);
        }

        let evidence_outcome = outcome_to_evidence(&outcome);
        writer
            .append(EventPayload::SetupStepFinished {
                phase,
                step_index: idx as u32,
                outcome: evidence_outcome.clone(),
                fixture_name: fixture_scope.map(|scope| scope.0.to_string()),
                check_id: fixture_scope.map(|scope| scope.1.to_string()),
            })
            .await?;

        if phase == StepPhase::Teardown && failed {
            let detail = error_detail.or_else(|| match &outcome {
                Outcome::Timeout => Some("action timed out".to_string()),
                Outcome::Error => Some("action returned an error".to_string()),
                Outcome::Ok => Some("judging action reported failure".to_string()),
                Outcome::Skipped { .. } => None,
            });
            cleanup.push(CleanupFailure {
                step: display_step_label(step, idx),
                outcome: evidence_outcome,
                detail,
            });
        }

        if aborted.is_none() {
            aborted = match outcome {
                Outcome::Timeout => Some(AbortReason::Timeout),
                Outcome::Error => Some(AbortReason::Environment),
                Outcome::Ok if failed => Some(AbortReason::Environment),
                Outcome::Ok | Outcome::Skipped { .. } => None,
            };
            if aborted.is_some() {
                failed_by = Some(display_step_label(step, idx));
            }
        }
    }

    if let Some(cb) = setup_browser {
        // Setup never keeps a video; skip the read + transfer entirely.
        let _ = cb.close(false, 0).await;
    }

    writer
        .append(EventPayload::SetupFinished {
            phase,
            aborted: aborted.is_some(),
            fixture_name: fixture_scope.map(|scope| scope.0.to_string()),
            check_id: fixture_scope.map(|scope| scope.1.to_string()),
        })
        .await?;
    if let Some(error) = stored_error {
        return Err(error);
    }
    Ok(LifecycleResult {
        aborted,
        failed_step: failed_by,
        cleanup,
    })
}

async fn append_setup_started(
    writer: &mut EvidenceWriter,
    phase: StepPhase,
    step: &Step,
    idx: usize,
    resolved_with: &serde_yml::Value,
    fixture_scope: Option<(&str, &str)>,
) -> Result<(), EngineError> {
    writer
        .append(EventPayload::SetupStepStarted {
            phase,
            step_index: idx as u32,
            uses: step.uses_name().to_string(),
            // Same honesty contract as the per-check tag (#192).
            layer: duhem_actions::layer_for_uses(step.uses_name()).map(str::to_string),
            with: with_to_evidence_map(resolved_with),
            fixture_name: fixture_scope.map(|scope| scope.0.to_string()),
            check_id: fixture_scope.map(|scope| scope.1.to_string()),
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
    fixture_scope: Option<(&'a str, &'a str)>,
}

async fn invoke_and_record(
    dispatcher: &dyn Dispatch,
    page: Option<&Page>,
    phase: StepPhase,
    idx: usize,
    resolved_with: &serde_yml::Value,
    invocation: SetupInvocation<'_>,
) -> Result<(Outcome, bool), EngineError> {
    let SetupInvocation {
        step,
        run,
        writer,
        child_env,
        fixture_scope,
    } = invocation;
    // The caller persisted `SetupStepStarted` before dispatch so slow
    // actions and gated skips share one honest lifecycle shape.
    let result = dispatcher.invoke(page, idx, resolved_with, child_env).await;
    let outcome = match &result {
        Ok(r) => r.outcome.clone(),
        Err(_) => Outcome::Error,
    };
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
                if let Some((fixture, _)) = fixture_scope {
                    run.record_fixture_output(fixture, id, local, v);
                } else {
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
                fixture_scope,
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
    };
    let judgment = fixture_scope
        .is_none()
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
    Ok((outcome, failed))
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
    fixture_scope: Option<(&str, &str)>,
) -> Result<(), EngineError> {
    use duhem_evidence::{BLOB_INLINE_THRESHOLD_BYTES, ObservationValue};
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
            fixture_name: fixture_scope.map(|scope| scope.0.to_string()),
            check_id: fixture_scope.map(|scope| scope.1.to_string()),
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::registry::Dispatch;
    use async_trait::async_trait;
    use duhem_actions::{ActionError, ActionResult, Outcome};
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StubAction {
        uses: &'static str,
        outcome: Outcome,
        outputs: Vec<(&'static str, serde_json::Value)>,
        invocations: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Dispatch for StubAction {
        fn uses(&self) -> &'static str {
            self.uses
        }
        fn requires_page(&self) -> bool {
            false
        }
        fn judges(&self) -> bool {
            self.outputs.iter().any(|(name, _)| *name == "satisfied")
        }
        async fn invoke(
            &self,
            _page: Option<&Page>,
            _step_index: usize,
            _with: &serde_yml::Value,
            _child_env: &BTreeMap<String, String>,
        ) -> Result<ActionResult, ActionError> {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            let mut r = match self.outcome {
                Outcome::Ok => ActionResult::ok(),
                Outcome::Error => ActionResult::error(),
                Outcome::Timeout => ActionResult::timeout(),
                Outcome::Skipped { ref reason } => ActionResult::skipped(reason.clone()),
            };
            for (k, v) in &self.outputs {
                r = r.with_output(k, v.clone());
            }
            Ok(r)
        }
    }

    async fn make_writer() -> (EvidenceWriter, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let store = duhem_evidence::SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .unwrap();
        let w = EvidenceWriter::begin(
            std::sync::Arc::new(store),
            duhem_evidence::new_run_id(),
            "x.yml",
            BTreeMap::new(),
        )
        .await
        .unwrap();
        (w, tmp)
    }

    fn step(id: Option<&str>, uses: &str) -> Step {
        Step {
            needs: vec![],
            id: id.map(String::from),
            description: None,
            condition: duhem_schema::StepCondition::Success,
            uses: Some(uses.to_string()),
            call: None,
            with: serde_yml::Value::Null,
            outputs: BTreeMap::new(),
            secret_outputs: Vec::new(),
            flow: None,
            flow_secrets: Vec::new(),
        }
    }

    fn conditioned_step(
        id: Option<&str>,
        uses: &str,
        condition: duhem_schema::StepCondition,
    ) -> Step {
        let mut step = step(id, uses);
        step.condition = condition;
        step
    }

    fn expr_condition(source: &str) -> duhem_schema::StepCondition {
        duhem_schema::StepCondition::Expr(duhem_schema::ExprStr::from_source(source).unwrap())
    }

    #[tokio::test]
    async fn value_conditions_gate_setup_on_true_and_false() {
        let (mut w, _tmp) = make_writer().await;
        let gated_calls = Arc::new(AtomicUsize::new(0));
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/seed",
            Box::new(StubAction {
                uses: "fake/seed",
                outcome: Outcome::Ok,
                outputs: vec![("n", serde_json::json!(2))],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        registry.insert(
            "fake/gated",
            Box::new(StubAction {
                uses: "fake/gated",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: gated_calls.clone(),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        let setup = vec![
            step(Some("existing"), "fake/seed"),
            conditioned_step(
                None,
                "fake/gated",
                expr_condition("$setup.existing.outputs.n > 0"),
            ),
            conditioned_step(
                None,
                "fake/gated",
                expr_condition("$setup.existing.outputs.n < 0"),
            ),
        ];
        let result = run_setup(&mut w, &registry, None, &mut run, &setup, &BTreeMap::new())
            .await
            .unwrap();
        assert!(result.aborted.is_none());
        assert_eq!(gated_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn value_conditions_gate_teardown() {
        let (mut w, _tmp) = make_writer().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/cleanup",
            Box::new(StubAction {
                uses: "fake/cleanup",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: calls.clone(),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        run.record_setup_output("existing", "n", crate::eval::Value::Int(1));
        let teardown = vec![
            conditioned_step(
                None,
                "fake/cleanup",
                expr_condition("$setup.existing.outputs.n == 1"),
            ),
            conditioned_step(
                None,
                "fake/cleanup",
                expr_condition("$setup.existing.outputs.n == 0"),
            ),
        ];
        let failures = run_teardown(
            &mut w,
            &registry,
            None,
            &mut run,
            &teardown,
            &BTreeMap::new(),
        )
        .await
        .unwrap();
        assert!(failures.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unresolvable_value_condition_is_an_error_not_a_skip() {
        let (mut w, _tmp) = make_writer().await;
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/gated",
            Box::new(StubAction {
                uses: "fake/gated",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        let setup = vec![conditioned_step(
            None,
            "fake/gated",
            expr_condition("$setup.missing.outputs.n > 0"),
        )];
        let error = run_setup(&mut w, &registry, None, &mut run, &setup, &BTreeMap::new())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("condition could not be evaluated"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn setup_publishes_outputs_into_run_state() {
        let (mut w, _tmp) = make_writer().await;
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/seed",
            Box::new(StubAction {
                uses: "fake/seed",
                outcome: Outcome::Ok,
                outputs: vec![("token", serde_json::json!("abc"))],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        let setup = vec![step(Some("warm"), "fake/seed")];
        let r = run_setup(&mut w, &registry, None, &mut run, &setup, &BTreeMap::new())
            .await
            .unwrap();
        assert!(r.aborted.is_none());
        assert_eq!(
            run.setup_outputs.get(&("warm".into(), "token".into())),
            Some(&crate::eval::Value::Str("abc".into())),
        );
    }

    #[tokio::test]
    async fn setup_aborts_on_first_error() {
        let (mut w, _tmp) = make_writer().await;
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/boom",
            Box::new(StubAction {
                uses: "fake/boom",
                outcome: Outcome::Error,
                outputs: vec![],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let after = Arc::new(AtomicUsize::new(0));
        registry.insert(
            "fake/tracker",
            Box::new(StubAction {
                uses: "fake/tracker",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: after.clone(),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        let setup = vec![step(None, "fake/boom"), step(None, "fake/tracker")];
        let r = run_setup(&mut w, &registry, None, &mut run, &setup, &BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(
            r.aborted,
            Some(AbortReason::Environment),
            "Outcome::Error should pin the cause to Environment"
        );
        assert_eq!(
            after.load(Ordering::SeqCst),
            0,
            "step after Error must not invoke"
        );
    }

    #[tokio::test]
    async fn setup_aborts_on_first_timeout() {
        // Mirrors the Error-side test for the Timeout branch of the
        // abort policy. A setup-step `Timeout` aborts setup, prevents
        // later setup steps from running, and pins the abort reason
        // to `Timeout` (which the engine maps to
        // `Inconclusive(Timeout)` on the run verdict).
        let (mut w, _tmp) = make_writer().await;
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/slow",
            Box::new(StubAction {
                uses: "fake/slow",
                outcome: Outcome::Timeout,
                outputs: vec![],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let after = Arc::new(AtomicUsize::new(0));
        registry.insert(
            "fake/tracker",
            Box::new(StubAction {
                uses: "fake/tracker",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: after.clone(),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        let setup = vec![step(None, "fake/slow"), step(None, "fake/tracker")];
        let r = run_setup(&mut w, &registry, None, &mut run, &setup, &BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(
            r.aborted,
            Some(AbortReason::Timeout),
            "Outcome::Timeout should pin the cause to Timeout"
        );
        assert_eq!(
            after.load(Ordering::SeqCst),
            0,
            "step after Timeout must not invoke"
        );
    }

    #[tokio::test]
    async fn setup_conditions_share_failure_state_but_keep_abort_policy() {
        let (mut w, _tmp) = make_writer().await;
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/boom",
            Box::new(StubAction {
                uses: "fake/boom",
                outcome: Outcome::Error,
                outputs: vec![],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let always_calls = Arc::new(AtomicUsize::new(0));
        registry.insert(
            "fake/always",
            Box::new(StubAction {
                uses: "fake/always",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: always_calls.clone(),
            }),
        );
        let failure_calls = Arc::new(AtomicUsize::new(0));
        registry.insert(
            "fake/failure",
            Box::new(StubAction {
                uses: "fake/failure",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: failure_calls.clone(),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        let setup = vec![
            step(Some("boom"), "fake/boom"),
            conditioned_step(None, "fake/always", duhem_schema::StepCondition::Always),
            conditioned_step(None, "fake/failure", duhem_schema::StepCondition::Failure),
        ];
        let result = run_setup(&mut w, &registry, None, &mut run, &setup, &BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(always_calls.load(Ordering::SeqCst), 1);
        assert_eq!(failure_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            result.aborted,
            Some(AbortReason::Environment),
            "opt-in cleanup does not soften setup's abort-the-run policy"
        );
    }

    #[tokio::test]
    async fn setup_failure_condition_skips_on_clean_sequence() {
        let (mut w, _tmp) = make_writer().await;
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/ok",
            Box::new(StubAction {
                uses: "fake/ok",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let failure_calls = Arc::new(AtomicUsize::new(0));
        registry.insert(
            "fake/failure",
            Box::new(StubAction {
                uses: "fake/failure",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: failure_calls.clone(),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        let setup = vec![
            step(None, "fake/ok"),
            conditioned_step(None, "fake/failure", duhem_schema::StepCondition::Failure),
        ];
        let result = run_setup(&mut w, &registry, None, &mut run, &setup, &BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(failure_calls.load(Ordering::SeqCst), 0);
        assert!(result.aborted.is_none());
    }

    #[tokio::test]
    async fn setup_judging_false_uses_the_shared_failure_predicate() {
        let (mut w, _tmp) = make_writer().await;
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/assert",
            Box::new(StubAction {
                uses: "fake/assert",
                outcome: Outcome::Ok,
                outputs: vec![("satisfied", serde_json::json!(false))],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let after_calls = Arc::new(AtomicUsize::new(0));
        registry.insert(
            "fake/after",
            Box::new(StubAction {
                uses: "fake/after",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: after_calls.clone(),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        let setup = vec![
            step(Some("precondition"), "fake/assert"),
            step(None, "fake/after"),
        ];
        let result = run_setup(&mut w, &registry, None, &mut run, &setup, &BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(after_calls.load(Ordering::SeqCst), 0);
        assert_eq!(result.aborted, Some(AbortReason::Environment));
    }

    #[tokio::test]
    async fn setup_engine_error_drains_always_and_failure_then_propagates() {
        let (mut w, _tmp) = make_writer().await;
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/ok",
            Box::new(StubAction {
                uses: "fake/ok",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let always_calls = Arc::new(AtomicUsize::new(0));
        registry.insert(
            "fake/always",
            Box::new(StubAction {
                uses: "fake/always",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: always_calls.clone(),
            }),
        );
        let failure_calls = Arc::new(AtomicUsize::new(0));
        registry.insert(
            "fake/failure",
            Box::new(StubAction {
                uses: "fake/failure",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: failure_calls.clone(),
            }),
        );
        let mut abort = step(Some("abort"), "fake/ok");
        abort.with = serde_yml::from_str("value: $setup.never.outputs.value").unwrap();
        let setup = vec![
            abort,
            conditioned_step(None, "fake/always", duhem_schema::StepCondition::Always),
            conditioned_step(None, "fake/failure", duhem_schema::StepCondition::Failure),
        ];
        let mut run = RunState::new(BTreeMap::new());
        let error = run_setup(&mut w, &registry, None, &mut run, &setup, &BTreeMap::new())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            EngineError::UnresolvedReference { reference, step, .. }
                if reference == "$setup.never.outputs.value" && step == "abort"
        ));
        assert_eq!(always_calls.load(Ordering::SeqCst), 1);
        assert_eq!(failure_calls.load(Ordering::SeqCst), 1);
    }
}
