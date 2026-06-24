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
use std::path::{Path, PathBuf};

use duhem_actions::Page;
use duhem_actions::{ActionError, Outcome, RunBrowser};
use duhem_evidence::{
    EventPayload, EvidenceWriter, StepOutcome, VerdictState, WriterError, new_run_id, run_started,
};
use duhem_judge::{
    AssertionOutcome, CheckOutcome, CheckVerdict, CriterionVerdict, InconclusiveCause, RunVerdict,
    aggregate_check, aggregate_criterion, aggregate_run,
};
use duhem_schema::{Check, Criterion, VerificationDefinition};
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
    /// A declared input's JSON value is outside the runtime `Value`
    /// model — e.g. a numeric literal that fits neither `i64` nor a
    /// finite `f64`. Surface this as an engine error instead of
    /// silently dropping the input (which would manifest later as a
    /// confusing `Inconclusive(MissingInput)`).
    #[error("input `{name}`: value is not representable as a runtime value")]
    InputUnrepresentable { name: String },
}

impl From<ActionError> for EngineError {
    fn from(e: ActionError) -> Self {
        EngineError::Browser(e.to_string())
    }
}

/// Predicate that decides whether the engine should execute a given
/// `(criterion_id, check_id)` pair. Used by the CLI `--filter` flag
/// (spec on issue #23) to skip checks the author isn't iterating on.
///
/// A filtered-out check is **skipped entirely**: no `StepStarted` /
/// `CheckFinished` events on the trace, no `AssertionOutcome` slot. A
/// criterion whose checks are all filtered out aggregates as empty →
/// `Inconclusive(EmptyAggregation)` per `aggregate_criterion(&[])`.
pub trait CheckFilter: Send + Sync {
    fn matches(&self, criterion_id: &str, check_id: &str) -> bool;
}

/// Engine-side run summary that carries the evidence location
/// alongside the verdict. Returned by [`Engine::run_with_metadata`];
/// thin convenience around [`Engine::run`] for callers (the CLI's
/// `--reporter json`, replay tooling) that need to point a downstream
/// reader at `trace.jsonl`.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub verdict: RunVerdict,
    pub run_id: String,
    pub run_dir: PathBuf,
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
    /// Optional check-level filter (spec on issue #23). When set, only
    /// matching checks execute; non-matching checks are skipped with
    /// no evidence emission.
    filter: Option<Box<dyn CheckFilter>>,
    /// Optional u64 seed for runtime entropy (spec on issue #33). When
    /// set, the per-run `$runtime.uuid()` value is derived
    /// deterministically from the seed instead of `Uuid::new_v4`. No
    /// other runtime semantics depend on this today.
    seed: Option<u64>,
    /// Skip `environment.up:` + readiness probe (the `--no-env-up`
    /// escape hatch on issue #50). The operator is presumed to have
    /// brought the SUT up already. Teardown still runs unless
    /// [`Engine::keep_env`] is also set.
    skip_env_up: bool,
    /// Skip `environment.down:` (the `--keep-env` debug flag on
    /// issue #50). Useful when an author wants the SUT to outlive the
    /// run for triage.
    keep_env: bool,
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
            filter: None,
            seed: None,
            skip_env_up: false,
            keep_env: false,
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

    /// Attach a [`CheckFilter`]. With a filter set, checks for which
    /// `matches(criterion_id, check_id)` returns `false` are skipped
    /// entirely — no events, no verdict slot. Spec on issue #23.
    pub fn with_filter(mut self, filter: impl CheckFilter + 'static) -> Self {
        self.filter = Some(Box::new(filter));
        self
    }

    /// Seed the runtime's entropy source so `$runtime.uuid()` is
    /// derived deterministically from `seed`. Two runs with the same
    /// seed see identical uuid output (spec on issue #33).
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Skip `environment.up:` + readiness probing. Used by the CLI's
    /// `--no-env-up` flag; useful when the operator brought the SUT
    /// up out-of-band. Teardown still runs unless [`Engine::keep_env`]
    /// is also set — that combination is the "do absolutely no
    /// lifecycle plumbing" debug shape.
    pub fn skip_env_up(mut self, skip: bool) -> Self {
        self.skip_env_up = skip;
        self
    }

    /// Skip `environment.down:`. Used by the CLI's `--keep-env` flag
    /// so an author can poke at the SUT after a failing run.
    pub fn keep_env(mut self, keep: bool) -> Self {
        self.keep_env = keep;
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
        Ok(self.run_with_metadata(def, inputs).await?.verdict)
    }

    /// Same as [`Engine::run`] but also returns the run identifier and
    /// the on-disk evidence directory. Used by the CLI's structured
    /// reporters and replay tooling (spec on issue #23).
    pub async fn run_with_metadata(
        &mut self,
        def: &VerificationDefinition,
        inputs: BTreeMap<String, serde_json::Value>,
    ) -> Result<RunOutcome, EngineError> {
        let mut input_values: BTreeMap<String, Value> = BTreeMap::new();
        for (k, v) in &inputs {
            let val = json_to_value(v)
                .ok_or_else(|| EngineError::InputUnrepresentable { name: k.clone() })?;
            input_values.insert(k.clone(), val);
        }
        let mut run_state = match self.seed {
            Some(s) => RunState::new_with_seed(input_values, s),
            None => RunState::new(input_values),
        };

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

        // Resolve the Verification Definition's directory so relative
        // `environment.up:` / `down:` paths anchor at the same place
        // an author would `cd` to before running the script by hand.
        let vd_dir: Option<PathBuf> = self
            .definition_path
            .as_deref()
            .and_then(|p| Path::new(p).parent().map(Path::to_path_buf));

        // `environment:` lifecycle precedes `setup:` (spec on
        // issue #50). On `up:` failure or `ready:` timeout we record
        // the verdict as `Inconclusive`, skip setup + criteria, and
        // delegate the teardown decision to `bring_environment_up`'s
        // `should_tear_down` signal (so a half-booted SUT or a
        // `--no-env-up` run still gets its `down:` invocation unless
        // `--keep-env` is also on).
        let mut env_should_tear_down = false;
        if let Some(env) = def.environment.as_ref() {
            let r = crate::engine::env::bring_environment_up(
                &mut writer,
                env,
                vd_dir.as_deref(),
                &run_state,
                self.skip_env_up,
            )
            .await?;
            env_should_tear_down = r.should_tear_down;
            if let Some(reason) = r.aborted {
                let verdict = RunVerdict {
                    state: VerdictState::Inconclusive(reason.cause()),
                    criteria: Vec::new(),
                };
                crate::engine::env::tear_environment_down(
                    &mut writer,
                    env,
                    vd_dir.as_deref(),
                    self.keep_env,
                    env_should_tear_down,
                )
                .await?;
                writer.append(EventPayload::RunFinished {
                    verdict: verdict.state,
                })?;
                writer.finish()?;
                return Ok(RunOutcome {
                    verdict,
                    run_id,
                    run_dir,
                });
            }
        }

        // Run-level `setup:` runs once before any criterion. Skipped
        // entirely when empty so the wire shape stays byte-identical
        // for setup-free Verification Definitions (issue #20).
        if !def.setup.is_empty() {
            let r = crate::engine::setup::run_setup(
                &mut writer,
                &self.registry,
                self.browser.as_ref(),
                &mut run_state,
                &def.setup,
            )
            .await?;
            if let Some(reason) = r.aborted {
                // Preserve the trigger on the verdict: a setup-step
                // `Timeout` surfaces as `Inconclusive(Timeout)`; an
                // `Error` (or any environmental precondition) as
                // `Inconclusive(EnvironmentError)`. Conflating the
                // two would lose useful telemetry on the trace.
                let verdict = RunVerdict {
                    state: VerdictState::Inconclusive(reason.cause()),
                    criteria: Vec::new(),
                };
                if let Some(env) = def.environment.as_ref() {
                    crate::engine::env::tear_environment_down(
                        &mut writer,
                        env,
                        vd_dir.as_deref(),
                        self.keep_env,
                        env_should_tear_down,
                    )
                    .await?;
                }
                writer.append(EventPayload::RunFinished {
                    verdict: verdict.state,
                })?;
                writer.finish()?;
                return Ok(RunOutcome {
                    verdict,
                    run_id,
                    run_dir,
                });
            }
        }

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
        if let Some(env) = def.environment.as_ref() {
            crate::engine::env::tear_environment_down(
                &mut writer,
                env,
                vd_dir.as_deref(),
                self.keep_env,
                env_should_tear_down,
            )
            .await?;
        }
        writer.append(EventPayload::RunFinished {
            verdict: run_verdict.state,
        })?;
        writer.finish()?;

        Ok(RunOutcome {
            verdict: run_verdict,
            run_id,
            run_dir,
        })
    }

    async fn run_criterion(
        &mut self,
        writer: &mut EvidenceWriter,
        run: &RunState,
        criterion: &Criterion,
    ) -> Result<CriterionVerdict, EngineError> {
        let mut check_verdicts: Vec<CheckVerdict> = Vec::new();
        for check in &criterion.checks {
            // Filtered-out checks emit no events and don't contribute
            // to verdict aggregation. A criterion with all checks
            // filtered out aggregates as empty → Inconclusive
            // (spec on issue #23).
            if let Some(f) = &self.filter
                && !f.matches(&criterion.id, &check.id)
            {
                continue;
            }
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
        | EvalCause::MissingSetupObservation { .. }
        | EvalCause::MissingInput(_)
        | EvalCause::MissingEnv(_)
        | EvalCause::MissingField(_) => InconclusiveCause::MissingObservation,
        EvalCause::UnknownRuntimeHelper(_)
        | EvalCause::TypeMismatch { .. }
        | EvalCause::NotNavigable { .. }
        | EvalCause::BadFormat(_)
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
        EvalCause::MissingSetupObservation { step, output } => {
            format!("missing_setup_observation({step}.{output})")
        }
        EvalCause::MissingInput(n) => format!("missing_input({n})"),
        EvalCause::MissingEnv(n) => format!("missing_env({n})"),
        EvalCause::UnknownRuntimeHelper(n) => format!("unknown_runtime_helper({n})"),
        EvalCause::TypeMismatch { lhs, rhs } => {
            format!("type_mismatch({}, {})", shape_wire(*lhs), shape_wire(*rhs))
        }
        EvalCause::InvalidPattern(msg) => format!("invalid_pattern({msg})"),
        EvalCause::MissingField(path) => format!("missing_field({path})"),
        EvalCause::NotNavigable { shape, segment } => {
            format!("not_navigable({}, {segment})", shape_wire(*shape))
        }
        EvalCause::BadFormat(msg) => format!("bad_format({msg})"),
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

pub(super) fn outcome_to_evidence(o: &Outcome) -> StepOutcome {
    match o {
        Outcome::Ok => StepOutcome::Ok,
        Outcome::Error => StepOutcome::Error,
        Outcome::Timeout => StepOutcome::Timeout,
    }
}

pub(super) fn with_to_evidence_map(v: &serde_yml::Value) -> BTreeMap<String, serde_json::Value> {
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
            filter: None,
            seed: None,
            skip_env_up: false,
            keep_env: false,
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
    async fn setup_output_threads_into_check_assertion() {
        // Spec on #20: `$setup.<id>.outputs.<x>` resolves in a check
        // assertion when a setup step produced output `<x>`. Inverse
        // (missing output) yields the run-level
        // Inconclusive(MissingObservation) path through the assertion.
        let (mut engine, _tmp) = engine_for_test();
        engine.register_test_action(Box::new(
            StubAction::new("fake/seed", Outcome::Ok).with_output("tok", serde_json::json!("abc")),
        ));
        let v = def(r#"
verification: t
setup:
  - id: warm
    uses: fake/seed
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - $setup.warm.outputs.tok == "abc"
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(verdict.state, VerdictState::Pass);
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
    async fn setup_error_aborts_run_with_inconclusive() {
        // Spec on #20: a setup step returning Outcome::Error aborts
        // setup, no criterion executes, the run verdict is
        // Inconclusive, and `SetupFinished { aborted: true }` is on
        // the trace.
        let (mut engine, tmp) = engine_for_test();
        engine.register_test_action(Box::new(StubAction::new("fake/boom", Outcome::Error)));
        let criterion_calls = Arc::new(AtomicUsize::new(0));
        let criterion_tracker = StubAction {
            uses: "fake/criterion",
            outcome: Outcome::Ok,
            outputs: Vec::new(),
            invocations: criterion_calls.clone(),
        };
        engine.register_test_action(Box::new(criterion_tracker));
        let v = def(r#"
verification: t
setup:
  - uses: fake/boom
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/criterion
        assertions:
          - "true"
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert!(
            matches!(verdict.state, VerdictState::Inconclusive(_)),
            "got {verdict:?}"
        );
        assert!(verdict.criteria.is_empty(), "no criterion should execute");
        assert_eq!(
            criterion_calls.load(Ordering::SeqCst),
            0,
            "criteria must not run after setup abort"
        );
        // Evidence carries `setup_finished { aborted: true }`.
        let events = read_only_run_events(&tmp);
        let saw_aborted = events.iter().any(|e| {
            matches!(
                &e.payload,
                duhem_evidence::EventPayload::SetupFinished { aborted: true }
            )
        });
        assert!(saw_aborted, "expected SetupFinished aborted=true");
    }

    #[tokio::test]
    async fn setup_timeout_aborts_run_with_inconclusive_timeout() {
        // Companion to `setup_error_aborts_run_with_inconclusive`:
        // the abort policy applies to `Outcome::Timeout` too, and the
        // verdict's `InconclusiveCause` distinguishes it from an
        // environmental setup failure.
        let (mut engine, tmp) = engine_for_test();
        engine.register_test_action(Box::new(StubAction::new("fake/slow", Outcome::Timeout)));
        let criterion_calls = Arc::new(AtomicUsize::new(0));
        let criterion_tracker = StubAction {
            uses: "fake/criterion",
            outcome: Outcome::Ok,
            outputs: Vec::new(),
            invocations: criterion_calls.clone(),
        };
        engine.register_test_action(Box::new(criterion_tracker));
        let v = def(r#"
verification: t
setup:
  - uses: fake/slow
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/criterion
        assertions:
          - "true"
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(
            verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
            "got {verdict:?}"
        );
        assert!(verdict.criteria.is_empty(), "no criterion should execute");
        assert_eq!(
            criterion_calls.load(Ordering::SeqCst),
            0,
            "criteria must not run after setup timeout"
        );
        let events = read_only_run_events(&tmp);
        let saw_aborted = events.iter().any(|e| {
            matches!(
                &e.payload,
                duhem_evidence::EventPayload::SetupFinished { aborted: true }
            )
        });
        assert!(saw_aborted, "expected SetupFinished aborted=true");
    }

    #[tokio::test]
    async fn empty_setup_emits_no_setup_events() {
        // Spec on #20: a definition with no `setup:` block produces a
        // byte-identical trace to today's setup-free definitions.
        let (mut engine, tmp) = engine_for_test();
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - "true"
"#);
        let _ = engine.run(&v, BTreeMap::new()).await.unwrap();
        let events = read_only_run_events(&tmp);
        let has_setup = events.iter().any(|e| {
            matches!(
                e.payload,
                duhem_evidence::EventPayload::SetupStarted { .. }
                    | duhem_evidence::EventPayload::SetupStepStarted { .. }
                    | duhem_evidence::EventPayload::SetupStepObservation { .. }
                    | duhem_evidence::EventPayload::SetupStepFinished { .. }
                    | duhem_evidence::EventPayload::SetupFinished { .. }
            )
        });
        assert!(!has_setup, "no Setup* events for empty setup block");
    }

    #[tokio::test]
    async fn missing_setup_output_in_assertion_is_inconclusive() {
        // Spec on #20: `$setup.s1.outputs.missing` evaluates to
        // Inconclusive(MissingObservation) — the
        // `MissingSetupObservation` evaluator cause maps to the
        // judge-level `MissingObservation` verdict.
        let (mut engine, tmp) = engine_for_test();
        engine.register_test_action(Box::new(StubAction::new("fake/seed", Outcome::Ok)));
        let v = def(r#"
verification: t
setup:
  - id: warm
    uses: fake/seed
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - $setup.warm.outputs.nope == "x"
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert!(matches!(
            verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation),
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
            detail.starts_with("missing_setup_observation("),
            "expected missing_setup_observation detail, got {detail:?}"
        );
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

    /// Trivial filter for engine-level tests: keeps an explicit set of
    /// `(criterion, check)` pairs.
    struct AllowList(Vec<(&'static str, &'static str)>);

    impl CheckFilter for AllowList {
        fn matches(&self, criterion_id: &str, check_id: &str) -> bool {
            self.0
                .iter()
                .any(|(c, k)| *c == criterion_id && *k == check_id)
        }
    }

    #[tokio::test]
    async fn filtered_out_check_emits_no_events_and_no_verdict_slot() {
        let (mut engine, tmp) = engine_for_test();
        engine.filter = Some(Box::new(AllowList(vec![("AC-1", "AC-1.1")])));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
      - id: AC-1.2
        assertions: ["false"]
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        // AC-1.2 is filtered out: it must not contribute a Fail.
        assert_eq!(verdict.state, VerdictState::Pass);
        let only_check = &verdict.criteria[0].checks;
        assert_eq!(only_check.len(), 1, "filtered check absent from verdict");
        assert_eq!(only_check[0].check_id, "AC-1.1");
        let events = read_only_run_events(&tmp);
        let saw_filtered_check = events.iter().any(|e| {
            matches!(&e.payload, duhem_evidence::EventPayload::CheckFinished { check_id, .. } if check_id == "AC-1.2")
        });
        assert!(!saw_filtered_check, "filtered check must emit no events");
    }

    #[tokio::test]
    async fn criterion_with_all_checks_filtered_is_inconclusive_empty() {
        // Spec (#23): a criterion whose checks are all filtered out
        // aggregates as empty → Inconclusive(EmptyAggregation), which
        // bubbles up to the run-level verdict.
        let (mut engine, _tmp) = engine_for_test();
        engine.filter = Some(Box::new(AllowList(vec![("AC-1", "AC-1.1")])));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
  - id: AC-2
    description: x
    checks:
      - id: AC-2.1
        assertions: ["true"]
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(
            verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation),
        );
        let ac2 = verdict
            .criteria
            .iter()
            .find(|c| c.criterion_id == "AC-2")
            .expect("AC-2 in verdict vector");
        assert_eq!(
            ac2.state,
            VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation),
        );
        assert!(ac2.checks.is_empty());
    }

    /// Spec on #33: a seeded run plumbs the seed through the engine
    /// to the evaluator, so `$runtime.uuid()` resolves to the
    /// deterministic uuid `RunState::new_with_seed(_, 42)` would
    /// produce. The CLI / RunState unit tests cover the input and
    /// output ends; this test covers the full engine path so any
    /// future change that forgets to thread `seed` to `RunState`
    /// flips the verdict here.
    #[tokio::test]
    async fn seeded_engine_evaluates_runtime_uuid_deterministically() {
        let expected = RunState::new_with_seed(BTreeMap::new(), 42).uuid;
        let yaml = format!(
            r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - $runtime.uuid() == "{expected}"
"#
        );
        let v = def(&yaml);
        let (mut e1, _tmp1) = engine_for_test();
        e1.seed = Some(42);
        let v1 = e1.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(
            v1.state,
            VerdictState::Pass,
            "seed=42 must evaluate $runtime.uuid() to the seeded literal"
        );

        // Sanity: a second seeded engine reaches the same verdict.
        let (mut e2, _tmp2) = engine_for_test();
        e2.seed = Some(42);
        let v2 = e2.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(v2.state, VerdictState::Pass);

        // Sanity: omitting the seed flips the verdict — `Uuid::new_v4`
        // colliding with the seeded literal would be a one-in-2^122
        // event, so this is a real determinism signal.
        let (mut e3, _tmp3) = engine_for_test();
        let v3 = e3.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(
            v3.state,
            VerdictState::Fail,
            "unseeded run should not accidentally match the seeded uuid"
        );
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

    /// Spec on #50: `environment.up:` exits 0, criteria run normally,
    /// and `environment.down:` is invoked after the criteria loop.
    /// Both scripts emit the `Env*` evidence events.
    #[tokio::test]
    async fn env_up_success_runs_criteria_and_invokes_down() {
        use std::fs::Permissions;
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;

        let (mut engine, tmp) = engine_for_test();
        let scripts_dir = tmp.path().join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        let up = scripts_dir.join("up.sh");
        let down = scripts_dir.join("down.sh");
        std::fs::write(&up, "#!/bin/sh\necho up\nexit 0\n").unwrap();
        std::fs::write(&down, "#!/bin/sh\necho down\nexit 0\n").unwrap();
        std::fs::set_permissions(&up, Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&down, Permissions::from_mode(0o755)).unwrap();

        // Place a VD next to scripts/ so the relative paths resolve.
        let vd_path = tmp.path().join("vd.yml");
        let mut f = std::fs::File::create(&vd_path).unwrap();
        writeln!(
            f,
            r#"
verification: env-up-success
environment:
  up: ./scripts/up.sh
  down: ./scripts/down.sh
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#
        )
        .unwrap();
        engine = engine.with_definition_path(vd_path.display().to_string());

        let def = duhem_schema::VerificationDefinition::from_yaml_str(
            &std::fs::read_to_string(&vd_path).unwrap(),
        )
        .unwrap();
        let outcome = engine
            .run_with_metadata(&def, BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(outcome.verdict.state, VerdictState::Pass);

        let events = duhem_evidence::Trace::open(&outcome.run_dir)
            .unwrap()
            .into_events();
        let kinds: Vec<&'static str> = events
            .iter()
            .map(|e| match &e.payload {
                duhem_evidence::EventPayload::EnvUpStarted { .. } => "env_up_started",
                duhem_evidence::EventPayload::EnvUpFinished { .. } => "env_up_finished",
                duhem_evidence::EventPayload::EnvDownStarted { .. } => "env_down_started",
                duhem_evidence::EventPayload::EnvDownFinished { .. } => "env_down_finished",
                duhem_evidence::EventPayload::RunFinished { .. } => "run_finished",
                _ => "other",
            })
            .collect();
        // Ordering: up_started → up_finished → ... → down_started →
        // down_finished → run_finished.
        let up_started = kinds.iter().position(|k| *k == "env_up_started").unwrap();
        let up_finished = kinds.iter().position(|k| *k == "env_up_finished").unwrap();
        let down_started = kinds.iter().position(|k| *k == "env_down_started").unwrap();
        let down_finished = kinds
            .iter()
            .position(|k| *k == "env_down_finished")
            .unwrap();
        let run_finished = kinds.iter().position(|k| *k == "run_finished").unwrap();
        assert!(up_started < up_finished);
        assert!(up_finished < down_started);
        assert!(down_started < down_finished);
        assert!(down_finished < run_finished);
    }

    /// Spec on #50: a non-zero `environment.up:` exit aborts the run
    /// with `Inconclusive(EnvironmentError)`; no setup or criterion
    /// runs; `environment.down:` is NOT invoked (nothing came up).
    #[tokio::test]
    async fn env_up_failure_yields_inconclusive_and_skips_down() {
        use std::fs::Permissions;
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;

        let (mut engine, tmp) = engine_for_test();
        let scripts_dir = tmp.path().join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        let up = scripts_dir.join("up.sh");
        let down = scripts_dir.join("down.sh");
        std::fs::write(&up, "#!/bin/sh\necho boom 1>&2\nexit 1\n").unwrap();
        std::fs::write(&down, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&up, Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&down, Permissions::from_mode(0o755)).unwrap();

        let vd_path = tmp.path().join("vd.yml");
        let mut f = std::fs::File::create(&vd_path).unwrap();
        writeln!(
            f,
            r#"
verification: env-up-fail
environment:
  up: ./scripts/up.sh
  down: ./scripts/down.sh
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#
        )
        .unwrap();
        engine = engine.with_definition_path(vd_path.display().to_string());

        let def = duhem_schema::VerificationDefinition::from_yaml_str(
            &std::fs::read_to_string(&vd_path).unwrap(),
        )
        .unwrap();
        let outcome = engine
            .run_with_metadata(&def, BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(
            outcome.verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
        );
        assert!(outcome.verdict.criteria.is_empty(), "no criterion runs");

        let events = duhem_evidence::Trace::open(&outcome.run_dir)
            .unwrap()
            .into_events();
        let saw_down = events.iter().any(|e| {
            matches!(
                &e.payload,
                duhem_evidence::EventPayload::EnvDownStarted { .. }
                    | duhem_evidence::EventPayload::EnvDownFinished { .. }
            )
        });
        assert!(!saw_down, "down: must not run when up: failed");
    }

    /// Spec on #50: `up:` exits 0, but `ready:` times out → run
    /// verdict `Inconclusive(Timeout)`, `down:` still runs.
    #[tokio::test]
    async fn env_ready_timeout_yields_inconclusive_and_runs_down() {
        use std::fs::Permissions;
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;

        let (mut engine, tmp) = engine_for_test();
        let scripts_dir = tmp.path().join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        let up = scripts_dir.join("up.sh");
        let down = scripts_dir.join("down.sh");
        std::fs::write(&up, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(&down, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&up, Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&down, Permissions::from_mode(0o755)).unwrap();

        let vd_path = tmp.path().join("vd.yml");
        let mut f = std::fs::File::create(&vd_path).unwrap();
        // Point at an unbound port via 127.0.0.1:1 — connect refuses
        // immediately; the timeout governs the polling loop.
        writeln!(
            f,
            r#"
verification: env-ready-timeout
environment:
  up: ./scripts/up.sh
  down: ./scripts/down.sh
  ready:
    http:
      url: http://127.0.0.1:1/healthz
      timeout: 600ms
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#
        )
        .unwrap();
        engine = engine.with_definition_path(vd_path.display().to_string());

        let def = duhem_schema::VerificationDefinition::from_yaml_str(
            &std::fs::read_to_string(&vd_path).unwrap(),
        )
        .unwrap();
        let outcome = engine
            .run_with_metadata(&def, BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(
            outcome.verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
        );

        let events = duhem_evidence::Trace::open(&outcome.run_dir)
            .unwrap()
            .into_events();
        let saw_ready_fail = events.iter().any(|e| {
            matches!(
                &e.payload,
                duhem_evidence::EventPayload::EnvReady { ok: false, .. }
            )
        });
        let saw_down = events.iter().any(|e| {
            matches!(
                &e.payload,
                duhem_evidence::EventPayload::EnvDownFinished { .. }
            )
        });
        assert!(saw_ready_fail, "expected EnvReady ok=false");
        assert!(saw_down, "down: should run after a ready timeout");
    }

    /// Spec on #50: `--no-env-up` (modeled here as `Engine::skip_env_up(true)`)
    /// skips `up:` and the readiness probe entirely. Criteria still
    /// run; `down:` does NOT run since `up:` never ran.
    #[tokio::test]
    async fn skip_env_up_bypasses_provisioning_but_runs_criteria() {
        use std::fs::Permissions;
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;

        let (mut engine, tmp) = engine_for_test();
        let scripts_dir = tmp.path().join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        let up = scripts_dir.join("up.sh");
        // Up script that would fail if it ran — confirms we skipped it.
        std::fs::write(&up, "#!/bin/sh\nexit 99\n").unwrap();
        std::fs::set_permissions(&up, Permissions::from_mode(0o755)).unwrap();

        let vd_path = tmp.path().join("vd.yml");
        let mut f = std::fs::File::create(&vd_path).unwrap();
        writeln!(
            f,
            r#"
verification: skip-env-up
environment:
  up: ./scripts/up.sh
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#
        )
        .unwrap();
        engine = engine
            .with_definition_path(vd_path.display().to_string())
            .skip_env_up(true);

        let def = duhem_schema::VerificationDefinition::from_yaml_str(
            &std::fs::read_to_string(&vd_path).unwrap(),
        )
        .unwrap();
        let outcome = engine
            .run_with_metadata(&def, BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(outcome.verdict.state, VerdictState::Pass);

        let events = duhem_evidence::Trace::open(&outcome.run_dir)
            .unwrap()
            .into_events();
        let saw_env = events.iter().any(|e| {
            matches!(
                &e.payload,
                duhem_evidence::EventPayload::EnvUpStarted { .. }
                    | duhem_evidence::EventPayload::EnvUpFinished { .. }
                    | duhem_evidence::EventPayload::EnvDownStarted { .. }
                    | duhem_evidence::EventPayload::EnvDownFinished { .. }
            )
        });
        assert!(!saw_env, "no Env* events when --no-env-up is on");
    }

    /// `--no-env-up` skips `up:` + readiness probing but still runs
    /// `down:` (unless `--keep-env` is also on) — the operator's
    /// expressed contract is "I brought it up; you tear it down".
    /// Verifies the CLI / runtime alignment that the doc on
    /// [`Engine::skip_env_up`] promises.
    #[tokio::test]
    async fn skip_env_up_still_runs_down_unless_keep_env_is_also_set() {
        use std::fs::Permissions;
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;

        let (mut engine, tmp) = engine_for_test();
        let scripts_dir = tmp.path().join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        let up = scripts_dir.join("up.sh");
        let down = scripts_dir.join("down.sh");
        // Up would fail if it ran; --no-env-up should keep it from
        // running.
        std::fs::write(&up, "#!/bin/sh\nexit 99\n").unwrap();
        std::fs::write(&down, "#!/bin/sh\necho down-ran\nexit 0\n").unwrap();
        std::fs::set_permissions(&up, Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&down, Permissions::from_mode(0o755)).unwrap();

        let vd_path = tmp.path().join("vd.yml");
        let mut f = std::fs::File::create(&vd_path).unwrap();
        writeln!(
            f,
            r#"
verification: skip-env-up-still-tears-down
environment:
  up: ./scripts/up.sh
  down: ./scripts/down.sh
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#
        )
        .unwrap();
        engine = engine
            .with_definition_path(vd_path.display().to_string())
            .skip_env_up(true);

        let def = duhem_schema::VerificationDefinition::from_yaml_str(
            &std::fs::read_to_string(&vd_path).unwrap(),
        )
        .unwrap();
        let outcome = engine
            .run_with_metadata(&def, BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(outcome.verdict.state, VerdictState::Pass);

        let events = duhem_evidence::Trace::open(&outcome.run_dir)
            .unwrap()
            .into_events();
        // No up-side events (skipped).
        let saw_up = events.iter().any(|e| {
            matches!(
                e.payload,
                duhem_evidence::EventPayload::EnvUpStarted { .. }
                    | duhem_evidence::EventPayload::EnvUpFinished { .. }
            )
        });
        assert!(!saw_up, "--no-env-up must skip up: + readiness events");
        // But down: ran.
        let saw_down_finished = events.iter().any(|e| {
            matches!(
                &e.payload,
                duhem_evidence::EventPayload::EnvDownFinished { exit_code: 0, .. }
            )
        });
        assert!(
            saw_down_finished,
            "--no-env-up alone should still invoke down: (operator's expressed contract)"
        );
    }

    /// Spec on #50: a VD without `environment:` produces a
    /// byte-shape-identical trace to today's setup-only definitions
    /// (no `Env*` events).
    #[tokio::test]
    async fn no_environment_block_emits_no_env_events() {
        let (mut engine, tmp) = engine_for_test();
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#);
        let _ = engine.run(&v, BTreeMap::new()).await.unwrap();
        let events = read_only_run_events(&tmp);
        let saw_env = events.iter().any(|e| {
            matches!(
                e.payload,
                duhem_evidence::EventPayload::EnvUpStarted { .. }
                    | duhem_evidence::EventPayload::EnvUpFinished { .. }
                    | duhem_evidence::EventPayload::EnvReady { .. }
                    | duhem_evidence::EventPayload::EnvDownStarted { .. }
                    | duhem_evidence::EventPayload::EnvDownFinished { .. }
            )
        });
        assert!(!saw_env, "VDs without `environment:` emit no Env* events");
    }
}
