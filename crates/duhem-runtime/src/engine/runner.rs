//! `Engine::run` — the v1 minimal step executor.
//!
//! Owns the "load → execute → evaluate → aggregate → emit" lifecycle
//! per the spec on issue #15. The function walks
//! `def.criteria × Criterion.checks × Check.steps` in order, lazily
//! opens a `CheckBrowser` only when a step in the check has a known
//! action, evaluates each `Assertion` via the shim + `eval()`, and
//! folds outcomes via `aggregate_check / aggregate_criterion /
//! aggregate_run` from `duhem-judge`. Errors only surface for
//! runtime-itself failures (browser launch refused, evidence not
//! writable); a failing artifact is a `RunVerdict::Fail`, not an
//! `Err`.
//!
//! Per-step error policy (alignment ratification on the issue):
//! `Outcome::Error` aborts the rest of the *check* (sibling checks
//! still run); the check's verdict is whatever `aggregate_check`
//! produces over the partial assertion outcomes. `Outcome::Timeout`
//! does *not* abort — assertions still evaluate and propagate
//! `Inconclusive(MissingObservation)` naturally.

use std::collections::BTreeMap;
use std::path::PathBuf;

use duhem_actions::{ActionError, Outcome, RunBrowser};
use duhem_evidence::{
    EventPayload, EvidenceWriter, StepOutcome, VerdictState, WriterError, new_run_id, run_started,
};
use duhem_judge::{
    AssertionOutcome, CheckOutcome, CheckVerdict, CriterionVerdict, InconclusiveCause, RunVerdict,
    aggregate_check, aggregate_criterion, aggregate_run,
};
use duhem_schema::{Check, Criterion, VerificationDefinition};
use playwright::api::Page;
use thiserror::Error;
use tracing::debug;

use crate::engine::context::{RunContext, RunState, json_to_value};
use crate::engine::registry::{ActionRegistry, default_registry};
use crate::engine::shim::assertion_to_expr;
use crate::engine::template::substitute_with;
use crate::eval::{EvalResult, InconclusiveCause as EvalCause, Value, eval};

/// Surfaces only "the runtime itself failed" cases. A failing
/// artifact yields `RunVerdict::Fail`, not `Err`.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Evidence directory or trace file could not be written.
    #[error("evidence: {0}")]
    Evidence(#[from] WriterError),
    /// Browser failed to launch when the run needed one. Carries the
    /// install-hint humanization from `RunBrowser::launch`.
    #[error("browser: {0}")]
    Browser(String),
}

impl From<ActionError> for EngineError {
    fn from(e: ActionError) -> Self {
        EngineError::Browser(e.to_string())
    }
}

/// The minimal step executor.
pub struct Engine {
    registry: ActionRegistry,
    evidence_root: PathBuf,
    /// Optional pre-launched browser. Set on production paths via
    /// `Engine::with_browser` so unit tests can construct an Engine
    /// without paying the Playwright launch cost.
    browser: Option<RunBrowser>,
    /// Caller-supplied path / identifier for the Verification
    /// Definition that's being run. Recorded as
    /// `manifest.definition_path` and the `run_started.verification_path`
    /// event field so evidence carries the actual on-disk source, not
    /// the human-readable `verification:` name. Set via
    /// [`Engine::with_definition_path`]; falls back to
    /// `def.verification` when absent.
    definition_path: Option<String>,
}

impl Engine {
    /// Build the v1 engine with the closed action catalog and a
    /// default evidence root of `.duhem/runs/`.
    pub fn new() -> Self {
        Self {
            registry: default_registry(),
            evidence_root: PathBuf::from(".duhem/runs"),
            browser: None,
            definition_path: None,
        }
    }

    /// Override the evidence root. Each run still lands in a fresh
    /// ULID-named subdirectory; this only changes the parent.
    pub fn with_evidence_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.evidence_root = root.into();
        self
    }

    /// Attach a pre-launched [`RunBrowser`]. The engine doesn't
    /// launch one on its own — the caller controls when the
    /// (heavyweight) Playwright process is started.
    pub fn with_browser(mut self, browser: RunBrowser) -> Self {
        self.browser = Some(browser);
        self
    }

    /// Record the source path / identifier of the Verification
    /// Definition for evidence. The CLI threads the file path here;
    /// programmatic callers can pass any stable identifier.
    pub fn with_definition_path(mut self, path: impl Into<String>) -> Self {
        self.definition_path = Some(path.into());
        self
    }

    /// Register a test-only dispatcher under a `Step.uses` key. The
    /// real catalog is closed at v1 (per the spec); this hook exists
    /// so the runner can be exercised in unit tests without booting
    /// a Playwright browser. Production callers go through
    /// [`Engine::new`].
    #[cfg(test)]
    pub(crate) fn register_test_action(&mut self, d: Box<dyn crate::engine::registry::Dispatch>) {
        self.registry.insert(d.uses(), d);
    }

    /// Walk every criterion / check / step and produce a `RunVerdict`.
    ///
    /// `inputs` is the typed, fully-resolved input set (one entry per
    /// declared `InputDecl`, coerced per its declared `InputType`).
    /// Per the typed-input-catalog spec, the CLI does the coercion
    /// and required/unknown-input checks before reaching here; the
    /// engine treats the map as authoritative.
    pub async fn run(
        &mut self,
        def: &VerificationDefinition,
        inputs: BTreeMap<String, serde_json::Value>,
    ) -> Result<RunVerdict, EngineError> {
        let input_values: BTreeMap<String, Value> = inputs
            .iter()
            .filter_map(|(k, v)| json_to_value(v).map(|val| (k.clone(), val)))
            .collect();
        let run_state = RunState::new(input_values);

        let run_id = new_run_id();
        let run_dir = self.evidence_root.join(&run_id);
        // Evidence records the *source* of the Verification
        // Definition. Prefer the caller-supplied `definition_path`
        // (the CLI threads the actual `.yml` path here). Fall back to
        // the human-readable `verification:` name only as a
        // last-resort identifier — it's not a file path, but it's
        // stable across runs and at least lets evidence be matched
        // back to a definition by name.
        let evidence_path = self
            .definition_path
            .clone()
            .unwrap_or_else(|| def.verification.clone());
        let mut writer = EvidenceWriter::new(&run_dir, &evidence_path)?;

        writer.append(run_started(evidence_path.clone(), inputs.clone()))?;

        let mut criterion_verdicts: Vec<CriterionVerdict> = Vec::new();
        for criterion in &def.criteria {
            let cv = self
                .run_criterion(&mut writer, &run_state, criterion)
                .await?;
            writer.append(EventPayload::CriterionFinished {
                criterion_id: criterion.id.clone(),
                verdict: cv.state,
            })?;
            criterion_verdicts.push(cv);
        }

        let run_verdict = aggregate_run(criterion_verdicts);
        writer.append(EventPayload::RunFinished {
            verdict: run_verdict.state,
        })?;
        writer.finish()?;

        Ok(run_verdict)
    }

    async fn run_criterion(
        &mut self,
        writer: &mut EvidenceWriter,
        run: &RunState,
        criterion: &Criterion,
    ) -> Result<CriterionVerdict, EngineError> {
        let mut check_verdicts: Vec<CheckVerdict> = Vec::new();
        for check in &criterion.checks {
            let cv = self.run_check(writer, run, &criterion.id, check).await?;
            writer.append(EventPayload::CheckFinished {
                check_id: check.id.clone(),
                verdict: cv.state,
            })?;
            check_verdicts.push(cv);
        }
        let state = aggregate_criterion(&check_verdicts);
        Ok(CriterionVerdict {
            criterion_id: criterion.id.clone(),
            state,
            checks: check_verdicts,
        })
    }

    async fn run_check(
        &mut self,
        writer: &mut EvidenceWriter,
        run: &RunState,
        criterion_id: &str,
        check: &Check,
    ) -> Result<CheckVerdict, EngineError> {
        let mut ctx = RunContext::new(run);

        // A `Step.uses` not in the registry means the step can't run
        // and its outputs don't exist; per spec, the check's
        // assertions all evaluate to Inconclusive(MissingObservation).
        let any_unknown = check
            .steps
            .iter()
            .any(|s| !self.registry.contains_key(s.uses.as_str()));

        // A step that requires a page (production wrapper around a
        // real `Action`) but has no browser attached is an
        // environment failure: the assertions can't be exercised, so
        // the check is Inconclusive(EnvironmentError) — not
        // accidentally Pass on a literal-only assertion in the same
        // check.
        let needs_browser = check.steps.iter().any(|s| {
            self.registry
                .get(s.uses.as_str())
                .map(|d| d.requires_page())
                .unwrap_or(false)
        });
        let browser_missing = needs_browser && self.browser.is_none();

        // Track per-check environment failures from open_check, too:
        // a browser was attached but allocating a context failed.
        let mut environment_failed = browser_missing;

        let mut check_browser = None;
        if !any_unknown
            && !browser_missing
            && !check.steps.is_empty()
            && let Some(b) = self.browser.as_ref()
        {
            match b.open_check().await {
                Ok(cb) => check_browser = Some(cb),
                Err(e) => {
                    debug!(error = %e, "open_check failed; check will surface as Inconclusive(EnvironmentError)");
                    environment_failed = true;
                }
            }
        }

        // Step execution loop. We emit `step_started` / `step_finished`
        // for every step the document declares, even when the step
        // can't run (unknown action, environment failure) — evidence
        // is more useful when it records what the author wrote, not
        // what the engine got around to invoking.
        let mut step_aborted = false;
        for (idx, step) in check.steps.iter().enumerate() {
            // Resolve template references in `with:` against whatever
            // context we have. Cheap and same-shape for every code
            // path, so we don't bifurcate evidence on it.
            let mut resolved_with = step.with.clone();
            substitute_with(&mut resolved_with, &ctx);

            writer.append(EventPayload::StepStarted {
                criterion_id: criterion_id.to_string(),
                check_id: check.id.clone(),
                step_index: idx as u32,
                uses: step.uses.clone(),
                with: with_to_evidence_map(&resolved_with),
            })?;

            let known = self.registry.contains_key(step.uses.as_str());
            let outcome = if !known || environment_failed || step_aborted {
                // Step can't run — emit a synthetic Error so evidence
                // carries the "not executed" signal alongside the
                // upstream cause via the assertion `detail`s below.
                Outcome::Error
            } else {
                let dispatcher = self
                    .registry
                    .get(step.uses.as_str())
                    .expect("known checked above");
                let page_ref: Option<&Page> = check_browser.as_ref().map(|cb| &cb.page);
                let result = dispatcher.invoke(page_ref, idx, &resolved_with).await;

                let outcome = match &result {
                    Ok(r) => r.outcome.clone(),
                    Err(_) => Outcome::Error,
                };

                if let Ok(r) = &result {
                    for (name, value) in &r.outputs {
                        if let Some(scalar) = json_to_value(value)
                            && let Some(id) = step.id.as_deref()
                        {
                            ctx.record_output(id, name, scalar);
                        }
                        writer.append_observation(idx as u32, name.clone(), value.clone())?;
                    }
                }

                outcome
            };

            writer.append(EventPayload::StepFinished {
                step_index: idx as u32,
                outcome: outcome_to_evidence(&outcome),
            })?;

            if matches!(outcome, Outcome::Error) {
                step_aborted = true;
            }
        }

        let mut assertion_outcomes: Vec<AssertionOutcome> = Vec::new();
        for (i, assertion) in check.assertions.iter().enumerate() {
            let expr = assertion_to_expr(assertion);
            let (state, detail) = if any_unknown {
                (
                    VerdictState::Inconclusive(InconclusiveCause::MissingObservation),
                    Some("unknown_action".to_string()),
                )
            } else if environment_failed {
                (
                    VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
                    Some(if browser_missing {
                        "browser_unavailable".to_string()
                    } else {
                        "check_browser_failed".to_string()
                    }),
                )
            } else {
                let r = eval(&expr, &ctx);
                let state = eval_to_state(&r);
                let detail = match &r {
                    EvalResult::Inconclusive(c) => Some(eval_cause_detail(c)),
                    _ => None,
                };
                (state, detail)
            };
            writer.append(EventPayload::AssertionEvaluated {
                check_id: check.id.clone(),
                assertion_index: i as u32,
                state,
                detail: detail.clone(),
            })?;
            assertion_outcomes.push(AssertionOutcome {
                assertion_index: i,
                state,
                detail,
            });
        }

        if let Some(cb) = check_browser {
            let _ = cb.close().await;
        }

        let outcome = CheckOutcome {
            check_id: check.id.clone(),
            assertions: assertion_outcomes,
        };
        Ok(aggregate_check(&outcome))
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

fn eval_to_state(r: &EvalResult) -> VerdictState {
    match r {
        EvalResult::True => VerdictState::Pass,
        EvalResult::False => VerdictState::Fail,
        EvalResult::Inconclusive(cause) => VerdictState::Inconclusive(map_eval_cause(cause)),
    }
}

fn map_eval_cause(c: &EvalCause) -> InconclusiveCause {
    match c {
        EvalCause::MissingObservation { .. }
        | EvalCause::MissingInput(_)
        | EvalCause::MissingEnv(_) => InconclusiveCause::MissingObservation,
        EvalCause::UnknownRuntimeHelper(_)
        | EvalCause::TypeMismatch { .. }
        | EvalCause::InvalidPattern(_) => InconclusiveCause::EnvironmentError,
    }
}

/// Format an evaluator-level `InconclusiveCause` for the
/// evidence-side `detail` field. The judge's `VerdictState` cause set
/// is intentionally coarse (`MissingObservation` /
/// `EnvironmentError` / etc.); the evidence-side string preserves the
/// specific reason so a reader can distinguish `missing_input(x)`
/// from `missing_observation(api.body)`, or
/// `invalid_pattern(...)` from `type_mismatch(str,int)`.
fn eval_cause_detail(c: &EvalCause) -> String {
    match c {
        EvalCause::MissingObservation { step, output } => {
            format!("missing_observation({step}.{output})")
        }
        EvalCause::MissingInput(n) => format!("missing_input({n})"),
        EvalCause::MissingEnv(n) => format!("missing_env({n})"),
        EvalCause::UnknownRuntimeHelper(n) => format!("unknown_runtime_helper({n})"),
        EvalCause::TypeMismatch { lhs, rhs } => {
            format!("type_mismatch({}, {})", shape_wire(*lhs), shape_wire(*rhs))
        }
        EvalCause::InvalidPattern(msg) => format!("invalid_pattern({msg})"),
    }
}

fn shape_wire(s: crate::eval::ValueShape) -> &'static str {
    use crate::eval::ValueShape;
    match s {
        ValueShape::Bool => "bool",
        ValueShape::Int => "int",
        ValueShape::Float => "float",
        ValueShape::Str => "str",
        ValueShape::Null => "null",
        ValueShape::Array => "array",
        ValueShape::Object => "object",
    }
}

fn outcome_to_evidence(o: &Outcome) -> StepOutcome {
    match o {
        Outcome::Ok => StepOutcome::Ok,
        Outcome::Error => StepOutcome::Error,
        Outcome::Timeout => StepOutcome::Timeout,
    }
}

fn with_to_evidence_map(v: &serde_yml::Value) -> BTreeMap<String, serde_json::Value> {
    match v {
        serde_yml::Value::Mapping(m) => m
            .iter()
            .filter_map(|(k, v)| {
                let key = k.as_str()?.to_string();
                let val = yml_to_json(v);
                Some((key, val))
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn yml_to_json(v: &serde_yml::Value) -> serde_json::Value {
    use serde_yml::Value as Y;
    match v {
        Y::Null => serde_json::Value::Null,
        Y::Bool(b) => serde_json::Value::Bool(*b),
        Y::Number(n) => serde_json::to_value(n).unwrap_or(serde_json::Value::Null),
        Y::String(s) => serde_json::Value::String(s.clone()),
        Y::Sequence(seq) => serde_json::Value::Array(seq.iter().map(yml_to_json).collect()),
        Y::Mapping(m) => serde_json::Value::Object(
            m.iter()
                .filter_map(|(k, v)| {
                    let key = k.as_str()?.to_string();
                    Some((key, yml_to_json(v)))
                })
                .collect(),
        ),
        Y::Tagged(t) => yml_to_json(&t.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::registry::Dispatch;
    use async_trait::async_trait;
    use duhem_actions::{ActionResult, Outcome};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// In-memory stub that ignores `page` and returns a configurable
    /// result. Test-only — kept under `#[cfg(test)]` per the spec.
    struct StubAction {
        uses: &'static str,
        outcome: Outcome,
        outputs: Vec<(&'static str, serde_json::Value)>,
        invocations: Arc<AtomicUsize>,
    }

    impl StubAction {
        fn new(uses: &'static str, outcome: Outcome) -> Self {
            Self {
                uses,
                outcome,
                outputs: Vec::new(),
                invocations: Arc::new(AtomicUsize::new(0)),
            }
        }
        fn with_output(mut self, k: &'static str, v: serde_json::Value) -> Self {
            self.outputs.push((k, v));
            self
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
        async fn invoke(
            &self,
            _page: Option<&Page>,
            _step_index: usize,
            _with: &serde_yml::Value,
        ) -> Result<ActionResult, ActionError> {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            let mut r = match self.outcome {
                Outcome::Ok => ActionResult::ok(),
                Outcome::Error => ActionResult::error(),
                Outcome::Timeout => ActionResult::timeout(),
            };
            for (k, v) in &self.outputs {
                r = r.with_output(k, v.clone());
            }
            Ok(r)
        }
    }

    fn engine_for_test() -> (Engine, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut e = Engine {
            registry: BTreeMap::new(),
            evidence_root: tmp.path().to_path_buf(),
            browser: None,
            definition_path: None,
        };
        // Clear default registry so each test composes its own.
        e.registry.clear();
        (e, tmp)
    }

    fn def(yaml: &str) -> VerificationDefinition {
        VerificationDefinition::from_yaml_str(yaml).expect("parse")
    }

    #[tokio::test]
    async fn unknown_action_yields_inconclusive_missing_observation_for_every_assertion() {
        let (mut engine, _tmp) = engine_for_test();
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: nope/unknown
        assertions:
          - "true"
          - "false"
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        // Both assertions should evaluate to MissingObservation, so
        // the check, criterion, and run all roll up to that cause.
        assert_eq!(
            verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation),
        );
        let check = &verdict.criteria[0].checks[0];
        assert!(matches!(
            check.state,
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation),
        ));
    }

    #[tokio::test]
    async fn step_outputs_are_threaded_into_sibling_assertions() {
        let (mut engine, _tmp) = engine_for_test();
        engine.register_test_action(Box::new(
            StubAction::new("fake/produce", Outcome::Ok).with_output("x", serde_json::json!(42)),
        ));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - id: s1
            uses: fake/produce
        assertions:
          - $steps.s1.outputs.x == 42
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(verdict.state, VerdictState::Pass);
    }

    #[tokio::test]
    async fn outcome_error_aborts_remaining_steps_in_same_check_only() {
        let (mut engine, _tmp) = engine_for_test();
        let after_err = Arc::new(AtomicUsize::new(0));
        // Step 1: returns Error. Step 2: tracks invocations. Step 2
        // should never be invoked because step 1 aborts the check.
        engine.register_test_action(Box::new(StubAction::new("fake/error", Outcome::Error)));
        let tracker = StubAction::new("fake/tracker", Outcome::Ok);
        let after_err_clone = tracker.invocations.clone();
        engine.register_test_action(Box::new(tracker));
        // And a sibling-check tracker so we can verify sibling
        // checks still run.
        let sibling = StubAction::new("fake/sibling", Outcome::Ok);
        let sibling_calls = sibling.invocations.clone();
        engine.register_test_action(Box::new(sibling));
        let _ = after_err;

        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/error
          - uses: fake/tracker
        assertions:
          - "true"
      - id: AC-1.2
        steps:
          - uses: fake/sibling
        assertions:
          - "true"
"#);
        let _ = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(
            after_err_clone.load(Ordering::SeqCst),
            0,
            "step after Error must not run"
        );
        assert_eq!(
            sibling_calls.load(Ordering::SeqCst),
            1,
            "sibling check should still run"
        );
    }

    /// Production-flavor stub: claims to require a page, so the
    /// engine treats the no-browser case as an environment failure.
    struct PageRequiringStub {
        uses: &'static str,
    }

    #[async_trait]
    impl Dispatch for PageRequiringStub {
        fn uses(&self) -> &'static str {
            self.uses
        }
        fn requires_page(&self) -> bool {
            true
        }
        async fn invoke(
            &self,
            _page: Option<&Page>,
            _step_index: usize,
            _with: &serde_yml::Value,
        ) -> Result<ActionResult, ActionError> {
            Ok(ActionResult::ok())
        }
    }

    /// Walk the run directory left behind by an `Engine::run` and
    /// return the parsed events for the (single) run inside it.
    fn read_only_run_events(tmp: &tempfile::TempDir) -> Vec<duhem_evidence::Event> {
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        assert_eq!(entries.len(), 1, "exactly one run dir");
        duhem_evidence::Trace::open(&entries[0])
            .unwrap()
            .into_events()
    }

    #[tokio::test]
    async fn missing_browser_for_page_step_yields_environment_error_not_pass() {
        let (mut engine, tmp) = engine_for_test();
        engine.register_test_action(Box::new(PageRequiringStub {
            uses: "ui/needs-page",
        }));
        // Literal `true` assertion next to a page-required step must
        // not slip past as Pass: there's no way to actually exercise
        // the artifact, so the check is Inconclusive(EnvironmentError).
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/needs-page
        assertions:
          - "true"
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(
            verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
        );
        let events = read_only_run_events(&tmp);
        let detail = events
            .iter()
            .find_map(|e| match &e.payload {
                duhem_evidence::EventPayload::AssertionEvaluated { detail, .. } => detail.clone(),
                _ => None,
            })
            .expect("AssertionEvaluated event with detail");
        assert_eq!(detail, "browser_unavailable");
    }

    #[tokio::test]
    async fn assertion_detail_preserves_specific_evaluator_cause() {
        let (mut engine, tmp) = engine_for_test();
        let v = def(r#"
verification: t
inputs:
  x:
    type: string
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - matches: { value: $inputs.x, pattern: "[" }
"#);
        let mut inputs = BTreeMap::new();
        inputs.insert("x".to_string(), serde_json::Value::String("ok".to_string()));
        let verdict = engine.run(&v, inputs).await.unwrap();
        // Verdict is coarsely EnvironmentError; evidence detail names
        // the specific invalid-pattern failure so an author can fix
        // the regex.
        assert!(matches!(
            verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::EnvironmentError)
        ));
        let events = read_only_run_events(&tmp);
        let detail = events
            .iter()
            .find_map(|e| match &e.payload {
                duhem_evidence::EventPayload::AssertionEvaluated { detail, .. } => detail.clone(),
                _ => None,
            })
            .unwrap_or_default();
        assert!(
            detail.starts_with("invalid_pattern("),
            "expected invalid_pattern detail, got {detail:?}"
        );
    }

    #[tokio::test]
    async fn unknown_action_short_circuit_still_emits_step_events() {
        let (mut engine, tmp) = engine_for_test();
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: nope/unknown
        assertions:
          - "true"
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert!(matches!(
            verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation),
        ));
        // The unknown step must still surface in evidence as
        // step_started + step_finished(Error), so a reader can see
        // *which* author-declared step couldn't run.
        let events = read_only_run_events(&tmp);
        let saw_started = events.iter().any(|e| {
            matches!(&e.payload, duhem_evidence::EventPayload::StepStarted { uses, .. } if uses == "nope/unknown")
        });
        let saw_finished_error = events.iter().any(|e| {
            matches!(
                &e.payload,
                duhem_evidence::EventPayload::StepFinished {
                    outcome: duhem_evidence::StepOutcome::Error,
                    ..
                }
            )
        });
        assert!(saw_started, "expected step_started for unknown action");
        assert!(
            saw_finished_error,
            "expected step_finished(Error) for unknown action"
        );
    }

    #[tokio::test]
    async fn typed_input_catalog_flows_end_to_end() {
        // Typed-input-catalog spec worked example: declared integer /
        // boolean / object inputs reach the evaluator as the right
        // scalar/structured shape — not as opaque strings.
        let (mut engine, _tmp) = engine_for_test();
        let v = def(r#"
verification: t
inputs:
  member_count: { type: integer }
  allow_invites: { type: boolean }
  feature_flags: { type: object }
criteria:
  - id: AC-1
    description: typed inputs reach the evaluator typed
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.member_count == 3
          - $inputs.allow_invites == true
          - type_check: { value: $inputs.feature_flags, is: object }
"#);
        let mut inputs = BTreeMap::new();
        inputs.insert("member_count".to_string(), serde_json::json!(3));
        inputs.insert("allow_invites".to_string(), serde_json::json!(true));
        inputs.insert(
            "feature_flags".to_string(),
            serde_json::json!({"dark_mode": true}),
        );
        let verdict = engine.run(&v, inputs).await.unwrap();
        assert_eq!(verdict.state, VerdictState::Pass);
    }

    #[tokio::test]
    async fn integer_input_does_not_compare_equal_to_string_literal() {
        // Before the typed catalog, `--inputs count=3` stored as
        // `Value::Str("3")` and `count == 3` was a type_mismatch
        // Inconclusive. With typed coercion this is now a real
        // numeric comparison.
        let (mut engine, _tmp) = engine_for_test();
        let v = def(r#"
verification: t
inputs:
  count: { type: integer }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.count == 3
"#);
        let mut inputs = BTreeMap::new();
        inputs.insert("count".to_string(), serde_json::json!(3));
        let verdict = engine.run(&v, inputs).await.unwrap();
        assert_eq!(verdict.state, VerdictState::Pass);
    }

    #[tokio::test]
    async fn run_verdict_preserves_document_order_of_criteria() {
        let (mut engine, _tmp) = engine_for_test();
        // No steps means no actions needed; assertions evaluate
        // straight from literals.
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: first
    checks:
      - id: AC-1.1
        assertions: ["true"]
  - id: AC-2
    description: second
    checks:
      - id: AC-2.1
        assertions: ["true"]
  - id: AC-3
    description: third
    checks:
      - id: AC-3.1
        assertions: ["false"]
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        let ids: Vec<&str> = verdict
            .criteria
            .iter()
            .map(|c| c.criterion_id.as_str())
            .collect();
        assert_eq!(ids, vec!["AC-1", "AC-2", "AC-3"]);
        // AC-3 fails → run fails (aggregate_run is being driven over
        // the document-ordered vector we just asserted on).
        assert_eq!(verdict.state, VerdictState::Fail);
    }
}
