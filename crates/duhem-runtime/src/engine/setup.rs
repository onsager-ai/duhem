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
//! `Timeout` surfaces as `Inconclusive(Timeout)`, a step that ran and
//! returned `Error` as `Inconclusive(MissingObservation)`, and an
//! unknown-action step or missing browser as
//! `Inconclusive(EnvironmentError)`.

use std::collections::BTreeMap;

use duhem_actions::RunBrowser;
use duhem_evidence::{EventPayload, EvidenceWriter, StepPhase};
use duhem_judge::InconclusiveCause;
use duhem_schema::Step;
use tracing::debug;

use crate::engine::context::RunState;
use crate::engine::for_each::{process_lifecycle_step, run_for_each_step};
use crate::engine::registry::ActionRegistry;
use crate::engine::runner::{CleanupFailure, EngineError};

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
    /// A lifecycle action ran but returned `Outcome::Error`, so the
    /// product behavior the step was meant to expose was not observed.
    ActionError,
    /// A step used an unknown `Step.uses`, or the runtime couldn't
    /// provision a lifecycle browser when one was required.
    Environment,
}

impl AbortReason {
    /// Map the abort trigger to a judge-level `InconclusiveCause` so
    /// `Engine::run` can short-circuit to a meaningful `RunVerdict`.
    pub fn cause(self) -> InconclusiveCause {
        match self {
            AbortReason::Timeout => InconclusiveCause::Timeout,
            AbortReason::ActionError => InconclusiveCause::MissingObservation,
            AbortReason::Environment => InconclusiveCause::EnvironmentError,
        }
    }
}

/// Where a lifecycle block (`setup:` / `teardown:` / fixture `up:` /
/// `down:`) was declared (#441 Part B). Drives both the evidence
/// scope fields (`fixture_name` / `check_id` / `criterion_id`) and
/// whether the block's judging actions gate its own abort — fixtures
/// keep their established Part-A behavior (no implicit-judgment
/// gating); every other level behaves like leaf `setup:`/`teardown:`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum HookScope<'a> {
    /// Leaf-level `setup:` / `teardown:` (#20 / #409). No scope
    /// fields — byte-identical to the pre-#441 wire shape.
    Leaf,
    /// Criterion-level `setup:` / `teardown:` (#441 Part B).
    Criterion(&'a str),
    /// Check-level `setup:` / `teardown:` (#441 Part B) — the check's
    /// *own* lifecycle block, not a fixture.
    Check(&'a str, &'a str),
    /// Fixture `up:` / `down:` for a consuming check (#449 Part A).
    Fixture(&'a str, &'a str),
}

impl<'a> HookScope<'a> {
    /// Evidence scope fields, in `(fixture_name, check_id,
    /// criterion_id)` order.
    pub(crate) fn evidence_fields(self) -> (Option<String>, Option<String>, Option<String>) {
        match self {
            HookScope::Leaf => (None, None, None),
            HookScope::Criterion(criterion) => (None, None, Some(criterion.to_string())),
            HookScope::Check(criterion, check) => {
                (None, Some(check.to_string()), Some(criterion.to_string()))
            }
            HookScope::Fixture(fixture, check) => {
                (Some(fixture.to_string()), Some(check.to_string()), None)
            }
        }
    }

    pub(crate) fn is_fixture(self) -> bool {
        matches!(self, HookScope::Fixture(..))
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
        HookScope::Leaf,
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
        HookScope::Leaf,
    )
    .await?
    .cleanup)
}

/// Criterion-level `setup:` (#441 Part B). `dispatched` mirrors the
/// leaf contract: `true` once any action was actually invoked, so the
/// caller knows whether to drain the paired `teardown:` even when
/// setup aborted partway through.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_criterion_setup(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    criterion_id: &str,
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
        HookScope::Criterion(criterion_id),
    )
    .await?;
    Ok(SetupResult {
        aborted: result.aborted,
        failed_step: result.failed_step,
    })
}

/// Criterion-level `teardown:` (#441 Part B). Same drain contract as
/// [`run_teardown`]: evidence-only, never replaces the criterion's
/// verdict.
pub(crate) async fn run_criterion_teardown(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    criterion_id: &str,
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
        HookScope::Criterion(criterion_id),
    )
    .await?
    .cleanup)
}

/// Check-level `setup:` (#441 Part B) — the check's *own* lifecycle
/// block, run before its `needs:` fixtures. Re-run on every retry
/// attempt, like fixtures.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_check_setup(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    criterion_id: &str,
    check_id: &str,
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
        HookScope::Check(criterion_id, check_id),
    )
    .await?;
    Ok(SetupResult {
        aborted: result.aborted,
        failed_step: result.failed_step,
    })
}

/// Check-level `teardown:` (#441 Part B), run after the check's
/// fixtures are torn down.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_check_teardown(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    criterion_id: &str,
    check_id: &str,
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
        HookScope::Check(criterion_id, check_id),
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
        HookScope::Fixture(fixture, check_id),
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
        HookScope::Fixture(fixture, check_id),
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
    scope: HookScope<'_>,
) -> Result<LifecycleResult, EngineError> {
    let (fixture_name, check_id, criterion_id) = scope.evidence_fields();
    writer
        .append(EventPayload::SetupStarted {
            phase,
            step_count: steps.len() as u32,
            fixture_name: fixture_name.clone(),
            check_id: check_id.clone(),
            criterion_id: criterion_id.clone(),
        })
        .await?;

    // Decide up front whether any step in this block needs a real
    // page. Mirrors the per-check logic in `Engine::run_check` so
    // setup behaves the same way on an env-failure path. A `for_each:`
    // step (#443 Tier 1) has no `uses:` of its own for the `call:`/
    // `steps:` body forms, so the scan looks at its (already
    // schema-expanded) body template instead — same iteration count
    // regardless of the runtime array, since the template is built
    // once per `for_each:`, not once per element.
    fn dispatchable_uses(s: &Step) -> Vec<&str> {
        if s.for_each.is_some() {
            s.for_each_body.iter().map(Step::uses_name).collect()
        } else {
            vec![s.uses_name()]
        }
    }
    let needs_browser = steps.iter().flat_map(dispatchable_uses).any(|uses| {
        registry
            .get(uses)
            .map(|d| d.requires_page())
            .unwrap_or(false)
    });
    let any_unknown = steps
        .iter()
        .flat_map(dispatchable_uses)
        .any(|uses| !registry.contains_key(uses));
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
        if step.for_each.is_some() {
            run_for_each_step(
                writer,
                registry,
                setup_browser.as_ref(),
                run,
                child_env,
                phase,
                scope,
                environment_failed,
                step,
                idx,
                dispatched,
                &mut aborted,
                &mut failed_by,
                &mut stored_error,
                &mut cleanup,
            )
            .await?;
            continue;
        }
        process_lifecycle_step(
            writer,
            registry,
            setup_browser.as_ref(),
            run,
            child_env,
            phase,
            scope,
            environment_failed,
            step,
            idx,
            None,
            dispatched,
            &mut aborted,
            &mut failed_by,
            &mut stored_error,
            &mut cleanup,
        )
        .await?;
    }

    if let Some(cb) = setup_browser {
        // Setup never keeps a video; skip the read + transfer entirely.
        let _ = cb.close(false, 0).await;
    }

    writer
        .append(EventPayload::SetupFinished {
            phase,
            aborted: aborted.is_some(),
            fixture_name,
            check_id,
            criterion_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::registry::Dispatch;
    use async_trait::async_trait;
    use duhem_actions::{ActionError, ActionResult, Outcome, Page};
    use duhem_evidence::StepOutcome;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StubAction {
        uses: &'static str,
        outcome: Outcome,
        outputs: Vec<(&'static str, serde_json::Value)>,
        invocations: Arc<AtomicUsize>,
    }

    struct PageAction;

    #[async_trait]
    impl Dispatch for PageAction {
        fn uses(&self) -> &'static str {
            "fake/page"
        }
        fn requires_page(&self) -> bool {
            true
        }
        async fn invoke(
            &self,
            _page: Option<&Page>,
            _step_index: usize,
            _with: &serde_yml::Value,
            _child_env: &BTreeMap<String, String>,
        ) -> Result<ActionResult, ActionError> {
            Ok(ActionResult::ok())
        }
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
            for_each: None,
            max: None,
            as_binding: None,
            steps: None,
            for_each_body: Vec::new(),
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
    async fn setup_action_error_aborts_with_missing_observation() {
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
            Some(AbortReason::ActionError),
            "Outcome::Error should pin the cause to ActionError"
        );
        assert_eq!(
            r.aborted.map(AbortReason::cause),
            Some(InconclusiveCause::MissingObservation)
        );
        assert_eq!(
            after.load(Ordering::SeqCst),
            0,
            "step after Error must not invoke"
        );
        let events = duhem_evidence::Trace::from_store(w.store().as_ref(), w.run_id())
            .await
            .unwrap()
            .into_events();
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::SetupStepFinished {
                outcome: StepOutcome::Error,
                detail: Some(detail),
                ..
            } if detail == "action `fake/boom` returned an error"
        )));
    }

    #[tokio::test]
    async fn setup_missing_browser_remains_environment_error() {
        let (mut w, _tmp) = make_writer().await;
        let registry: ActionRegistry =
            BTreeMap::from([("fake/page", Box::new(PageAction) as Box<dyn Dispatch>)]);
        let mut run = RunState::new(BTreeMap::new());
        let result = run_setup(
            &mut w,
            &registry,
            None,
            &mut run,
            &[step(None, "fake/page")],
            &BTreeMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            result.aborted.map(AbortReason::cause),
            Some(InconclusiveCause::EnvironmentError)
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
        let events = duhem_evidence::Trace::from_store(w.store().as_ref(), w.run_id())
            .await
            .unwrap()
            .into_events();
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::SetupStepFinished {
                outcome: StepOutcome::Timeout,
                detail: Some(detail),
                ..
            } if detail == "action `fake/slow` timed out"
        )));
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
            Some(AbortReason::ActionError),
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
