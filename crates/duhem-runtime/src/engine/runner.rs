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
use std::sync::Arc;
use std::time::Duration;

use duhem_actions::Page;
use duhem_actions::{Outcome, RunBrowser};
use duhem_evidence::{
    EventPayload, EvidenceWriter, RunScope, SqliteStore, Store, StoreError, VerdictState,
    new_run_id, project_db_path, run_started,
};
use duhem_judge::{
    AssertionOutcome, CheckOutcome, CheckVerdict, CriterionVerdict, InconclusivePolicy, RunVerdict,
    aggregate_check, aggregate_criterion, aggregate_run, apply_inconclusive_policy,
};
use duhem_schema::{Check, Criterion, RetryBackoff, RetryPolicy, VerificationDefinition};
use tracing::debug;

pub use crate::engine::outcome::{
    CapturedArtifact, CheckFailure, CheckFilter, EngineError, FailedAssertion, RunOutcome,
};
pub(crate) use crate::engine::outcome::{
    StepEvidence, append_implicit_judgment, evaluate_explicit_assertions,
    implicit_judgment_outcomes, step_label,
};

use crate::engine::capture::{CapturePolicy, TargetLocator, finalize_capture, target_from_step};
use crate::engine::context::{RunContext, RunState, json_to_value};
use crate::engine::registry::{ActionRegistry, default_registry};
use crate::engine::template::substitute_with;
use crate::engine::translate::{
    RETRY_BACKOFF_BASE, apply_default_within, check_is_retryable, outcome_to_evidence, retry_delay,
    with_to_evidence_map,
};
use crate::eval::Value;

/// The minimal step executor.
pub struct Engine {
    registry: ActionRegistry,
    /// The evidence store this engine writes runs into. `None` until
    /// first use — [`Engine::run_with_metadata`] lazily opens the
    /// working copy's default store (`project_db_path(cwd)`), so
    /// zero-config runs work; the CLI passes an explicit store via
    /// [`Engine::with_store`].
    store: Option<Arc<dyn Store>>,
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
    /// Optional caller-supplied run id (#189). Fixtures and tests use
    /// it for deterministic run URLs; production runs leave it unset
    /// and mint a fresh ULID. A collision with an existing run fails
    /// loudly at `begin_run` (run ids are primary keys).
    run_id: Option<String>,
    /// Scoping + provenance for recorded runs (#190): the project
    /// hint and the `verifier VERIFIES target` coordinates. Default
    /// (all-`None`) records an unattributed run; #191's resolution
    /// ladder populates it.
    scope: RunScope,
    /// Skip `environment.up:` + readiness probe (the `--no-env-up`
    /// escape hatch on issue #50). The operator is presumed to have
    /// brought the SUT up already. Teardown still runs unless
    /// [`Engine::keep_env`] is also set.
    skip_env_up: bool,
    /// Skip `environment.down:` (the `--keep-env` debug flag on
    /// issue #50). Useful when an author wants the SUT to outlive the
    /// run for triage.
    keep_env: bool,
    /// `$env.<key>` whitelist seed for this run (spec #68). Empty by
    /// default — env access from assertions is opt-in. The CLI seeds
    /// this from the selected named environment's string-valued keys.
    env: BTreeMap<String, String>,
    /// Names the leaf declared under `inherits:` (spec #135). Used to
    /// turn a generic unresolved `$inputs.<name>` into the loud,
    /// specific "declared `inherits:` but nothing provides it" error
    /// with the suite/--inputs remedy. Empty for a leaf with no
    /// `inherits:` block, so non-inheriting runs keep today's behavior.
    inherited: std::collections::HashSet<String>,
    /// Manifest `defaults.timeout` (spec #66): per-step `within:`
    /// fallback. A step's own `within:` wins; absent here, the action's
    /// built-in `DEFAULT_WITHIN` (5s) applies. `None` keeps today's
    /// behavior.
    default_within: Option<Duration>,
    /// Manifest `defaults.inconclusive_policy` (spec #66). `Block`
    /// (today's behavior) is the default.
    inconclusive_policy: InconclusivePolicy,
    /// Manifest `defaults.retry` (spec #66). `None` = no retries.
    retry: Option<RetryPolicy>,
    /// Retry backoff base; tests drop it to zero. Production:
    /// [`RETRY_BACKOFF_BASE`].
    retry_backoff_base: Duration,
    /// Failure-evidence capture posture (spec #202).
    capture: CapturePolicy,
}

impl Engine {
    /// Build the v1 engine with the closed action catalog. The
    /// evidence store defaults to the working copy's project DB
    /// (opened lazily on first run) unless [`Engine::with_store`]
    /// supplies one.
    pub fn new() -> Self {
        Self {
            registry: default_registry(),
            store: None,
            browser: None,
            definition_path: None,
            filter: None,
            seed: None,
            run_id: None,
            scope: RunScope::default(),
            skip_env_up: false,
            keep_env: false,
            env: BTreeMap::new(),
            inherited: std::collections::HashSet::new(),
            default_within: None,
            inconclusive_policy: InconclusivePolicy::Block,
            retry: None,
            retry_backoff_base: RETRY_BACKOFF_BASE,
            capture: CapturePolicy::default(),
        }
    }

    /// Apply a manifest's `defaults:` block (spec #66): the per-step
    /// `within:` fallback (`timeout`), the inconclusive policy, and the
    /// retry posture. `defaults.environment` is not consumed here (its
    /// `environments:` lookup is out of scope). Absent sub-keys leave
    /// today's behavior in place.
    pub fn with_defaults(mut self, defaults: &duhem_schema::ManifestDefaults) -> Self {
        self.default_within = defaults.timeout.map(Duration::from);
        self.retry = defaults.retry;
        self.inconclusive_policy = match defaults.inconclusive_policy {
            Some(duhem_schema::InconclusivePolicy::Block) | None => InconclusivePolicy::Block,
            Some(duhem_schema::InconclusivePolicy::Warn) => InconclusivePolicy::Warn,
            Some(duhem_schema::InconclusivePolicy::Pass) => InconclusivePolicy::Pass,
        };
        self
    }

    /// Attach the evidence store this engine writes runs into. The
    /// CLI resolves the store (default project DB or `--db` override)
    /// and threads it here; without one, the engine lazily opens the
    /// working copy's default store on first run.
    pub fn with_store(mut self, store: Arc<dyn Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// Attach a pre-launched [`RunBrowser`]. The engine doesn't
    /// launch one on its own — the caller controls when the
    /// (heavyweight) Playwright process is started.
    pub fn with_browser(mut self, browser: RunBrowser) -> Self {
        self.browser = Some(browser);
        self
    }

    /// Set the failure-evidence capture posture (spec #202). Default
    /// is [`CapturePolicy::OnFailure`].
    pub fn with_capture(mut self, capture: CapturePolicy) -> Self {
        self.capture = capture;
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

    /// Pin the run id instead of minting a fresh ULID (#189). For
    /// fixtures and tests that need deterministic run URLs; colliding
    /// with an existing run fails at `begin_run`.
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    /// Attach scoping + provenance to recorded runs (#190): the
    /// project hint and `verifier_repo@sha VERIFIES target_repo@sha`.
    /// The identity-resolution ladder that computes these is #191's;
    /// this only records what the caller resolved.
    pub fn with_scope(mut self, scope: RunScope) -> Self {
        self.scope = scope;
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

    /// Seed the `$env.<key>` whitelist for this run (spec #68). The CLI
    /// passes the selected named environment's string-valued keys here;
    /// `$env.<key>` resolves against this map. Empty by default, so
    /// runs without a selected environment keep today's behavior (no
    /// `$env` access).
    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Declare the leaf's inherited input names (spec #135). When a
    /// referenced `$inputs.<name>` for one of these names resolves to
    /// nothing, the run fails with a loud, specific error naming it as
    /// inherited and pointing at the suite / `--inputs` remedy, instead
    /// of a generic deep failure. The CLI threads `def.inherits` here.
    pub fn with_inherited(mut self, names: impl IntoIterator<Item = String>) -> Self {
        self.inherited = names.into_iter().collect();
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
        // Loud, specific guard for unresolved inherited inputs (spec
        // #135): an `inherits:` name that the chain bound nothing for
        // (no manifest environment, no `--inputs`) fails here — before
        // any browser launch or network call — naming the input as
        // inherited and giving the suite / `--inputs` remedy, instead
        // of surfacing later as a generic deep failure.
        if !self.inherited.is_empty()
            && let Some(name) =
                crate::engine::inherit::first_unbound_inherited(def, &self.inherited, &input_values)
        {
            return Err(EngineError::UnresolvedInheritedInput { name });
        }
        let mut run_state = match self.seed {
            Some(s) => RunState::new_with_seed(input_values, s),
            None => RunState::new(input_values),
        };
        // Seed the `$env.<key>` whitelist from the selected named
        // environment (spec #68). Empty unless the CLI set it, so
        // environment-free runs keep today's "no `$env` access"
        // behavior.
        if !self.env.is_empty() {
            run_state = run_state.with_env(self.env.clone());
        }

        // Resolve the store: the caller-supplied one (CLI), else the
        // working copy's default project DB — so zero-config
        // programmatic runs still land somewhere sensible.
        let store: Arc<dyn Store> = match &self.store {
            Some(s) => s.clone(),
            None => {
                let cwd = std::env::current_dir().map_err(StoreError::Io)?;
                let db = project_db_path(&cwd)?;
                let opened: Arc<dyn Store> = Arc::new(SqliteStore::open(db).await?);
                self.store = Some(opened.clone());
                opened
            }
        };

        let run_id = self.run_id.clone().unwrap_or_else(new_run_id);
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
        let mut writer = EvidenceWriter::begin_scoped(
            store,
            &run_id,
            &evidence_path,
            inputs.clone(),
            self.scope.clone(),
        )
        .await?;

        writer
            .append(run_started(evidence_path.clone(), inputs.clone()))
            .await?;

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
                writer
                    .append(EventPayload::RunFinished {
                        verdict: verdict.state,
                    })
                    .await?;
                writer.finish().await?;
                return Ok(RunOutcome {
                    verdict,
                    run_id,
                    // The run aborted before any criterion executed, so
                    // there are no per-assertion failures or warnings
                    // to surface.
                    failures: Vec::new(),
                    warnings: Vec::new(),
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
                writer
                    .append(EventPayload::RunFinished {
                        verdict: verdict.state,
                    })
                    .await?;
                writer.finish().await?;
                return Ok(RunOutcome {
                    verdict,
                    run_id,
                    // The run aborted before any criterion executed, so
                    // there are no per-assertion failures or warnings
                    // to surface.
                    failures: Vec::new(),
                    warnings: Vec::new(),
                });
            }
        }

        let mut criterion_verdicts: Vec<CriterionVerdict> = Vec::new();
        let mut failures: Vec<CheckFailure> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        for criterion in &def.criteria {
            let cv = self
                .run_criterion(
                    &mut writer,
                    &run_state,
                    criterion,
                    &mut failures,
                    &mut warnings,
                )
                .await?;
            writer
                .append(EventPayload::CriterionFinished {
                    criterion_id: criterion.id.clone(),
                    verdict: cv.state,
                })
                .await?;
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
        writer
            .append(EventPayload::RunFinished {
                verdict: run_verdict.state,
            })
            .await?;
        writer.finish().await?;

        Ok(RunOutcome {
            verdict: run_verdict,
            run_id,
            failures,
            warnings,
        })
    }

    async fn run_criterion(
        &mut self,
        writer: &mut EvidenceWriter,
        run: &RunState,
        criterion: &Criterion,
        failures: &mut Vec<CheckFailure>,
        warnings: &mut Vec<String>,
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
            let cv = self
                .run_check_with_retry(writer, run, &criterion.id, check, failures)
                .await?;
            writer
                .append(EventPayload::CheckFinished {
                    check_id: check.id.clone(),
                    verdict: cv.state,
                })
                .await?;
            check_verdicts.push(cv);
        }
        // Criterion-level aggregation, then the manifest's
        // `inconclusive_policy` lens (spec #66). `block` (the default)
        // leaves the verdict untouched; `warn`/`pass` soften a
        // criterion-level `inconclusive` to `pass`, with `warn` also
        // recording a run-summary warning. Per-check verdicts (and the
        // CheckFinished events above) keep their raw `inconclusive`.
        let raw_state = aggregate_criterion(&check_verdicts);
        let (state, warning) =
            apply_inconclusive_policy(&criterion.id, raw_state, self.inconclusive_policy);
        if let Some(w) = warning {
            warnings.push(w);
        }
        Ok(CriterionVerdict {
            criterion_id: criterion.id.clone(),
            state,
            checks: check_verdicts,
        })
    }

    /// Run one check, re-running it from step 0 when `defaults.retry`
    /// is set and the verdict is retry-eligible (spec #66; see
    /// [`check_is_retryable`]). Each attempt re-emits the check's
    /// step / assertion events; only the final attempt's failing
    /// assertions stay in `failures`.
    async fn run_check_with_retry(
        &mut self,
        writer: &mut EvidenceWriter,
        run: &RunState,
        criterion_id: &str,
        check: &Check,
        failures: &mut Vec<CheckFailure>,
    ) -> Result<CheckVerdict, EngineError> {
        let max = self.retry.map(|r| r.max).unwrap_or(0);
        let backoff = self
            .retry
            .map(|r| r.backoff)
            .unwrap_or(RetryBackoff::Exponential);
        let mut attempt: u32 = 0;
        loop {
            // Discard any failures a prior (retried) attempt left behind
            // so only the final attempt's detail reaches the reporter.
            let failures_mark = failures.len();
            let cv = self
                .run_check(writer, run, criterion_id, check, failures)
                .await?;
            if attempt < max && check_is_retryable(cv.state) {
                failures.truncate(failures_mark);
                attempt += 1;
                let delay = retry_delay(self.retry_backoff_base, backoff, attempt);
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                continue;
            }
            return Ok(cv);
        }
    }

    async fn run_check(
        &mut self,
        writer: &mut EvidenceWriter,
        run: &RunState,
        criterion_id: &str,
        check: &Check,
        failures: &mut Vec<CheckFailure>,
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
        let mut targets: Vec<TargetLocator> = Vec::new();
        // Per-step evidence (resolved `with:` + outputs) for implicit
        // judgment (#280). Empty = the step didn't run.
        let mut step_evidence = vec![StepEvidence::empty(); check.steps.len()];
        for (idx, step) in check.steps.iter().enumerate() {
            // Resolve template references in `with:` against whatever
            // context we have. Cheap and same-shape for every code
            // path, so we don't bifurcate evidence on it.
            let mut resolved_with = step.with.clone();
            if let Err(u) = substitute_with(&mut resolved_with, &ctx) {
                return Err(EngineError::UnresolvedReference {
                    reference: u.reference,
                    context: u
                        .context
                        .map(|c| format!(" (evaluating `{c}`)"))
                        .unwrap_or_default(),
                    step: step_label(step, idx),
                });
            }
            // Manifest `defaults.timeout` (spec #66): fill the step's
            // `within:` when it doesn't declare its own. A per-step
            // `within:` already in the payload wins; this only fills the
            // gap. With no manifest default, the action's built-in
            // `DEFAULT_WITHIN` (5s) remains the last resort.
            if let Some(default) = self.default_within {
                apply_default_within(&mut resolved_with, default);
            }

            // Collect ui/assert-element targets for the element-highlight
            // overlay (spec #214) — but only for steps that actually run.
            // A skipped step (env failure, an earlier abort, unknown
            // action) never "looked" for anything, so recording its
            // locator would be misleading evidence.
            let will_run = !environment_failed
                && !step_aborted
                && self.registry.contains_key(step.uses.as_str());
            if will_run && let Some(t) = target_from_step(&step.uses, &resolved_with) {
                targets.push(t);
            }

            writer
                .append(EventPayload::StepStarted {
                    criterion_id: criterion_id.to_string(),
                    check_id: check.id.clone(),
                    step_index: idx as u32,
                    uses: step.uses.clone(),
                    // Honest evidence (#192): the layer comes from the
                    // executed action's catalog family, never from
                    // parsing intent; out-of-catalog `uses` stays
                    // untagged.
                    layer: duhem_actions::layer_for_uses(&step.uses).map(str::to_string),
                    with: with_to_evidence_map(&resolved_with),
                })
                .await?;

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
                    };
                }

                outcome
            };

            writer
                .append(EventPayload::StepFinished {
                    step_index: idx as u32,
                    outcome: outcome_to_evidence(&outcome),
                })
                .await?;

            if matches!(outcome, Outcome::Error) {
                step_aborted = true;
            }
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
            &step_evidence,
            any_unknown,
            environment_failed,
            browser_missing,
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

        // Failure-evidence capture (spec #202): the browser is still
        // open and the failure set is known, so this is the one spot
        // where "what did the page look like" can be recorded. Rides
        // the `step_observation` blob channel under the reserved
        // `capture/` prefix — the dashboard's existing artifact
        // pipeline picks it up with no reader/SPA changes.
        let mut captures: Vec<CapturedArtifact> = Vec::new();
        if let Some(cb) = check_browser {
            let wants_capture = match self.capture {
                CapturePolicy::Off => false,
                CapturePolicy::Always => true,
                CapturePolicy::OnFailure => !failed.is_empty(),
            };
            let last_step = check.steps.len().saturating_sub(1) as u32;
            captures = finalize_capture(writer, cb, wants_capture, last_step, &targets).await;
        }

        let outcome = CheckOutcome {
            check_id: check.id.clone(),
            assertions: assertion_outcomes,
        };
        let verdict = aggregate_check(&outcome);
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
        Ok(verdict)
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::registry::Dispatch;
    use async_trait::async_trait;
    use duhem_actions::{ActionError, ActionResult, Outcome};
    use duhem_judge::InconclusiveCause;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    /// In-memory stub that ignores `page` and returns a configurable
    /// result. Test-only — kept under `#[cfg(test)]` per the spec.
    struct StubAction {
        uses: &'static str,
        outcome: Outcome,
        outputs: Vec<(&'static str, serde_json::Value)>,
        invocations: Arc<AtomicUsize>,
        judges: bool,
    }

    impl StubAction {
        fn new(uses: &'static str, outcome: Outcome) -> Self {
            Self {
                uses,
                outcome,
                outputs: Vec::new(),
                invocations: Arc::new(AtomicUsize::new(0)),
                judges: false,
            }
        }
        fn with_output(mut self, k: &'static str, v: serde_json::Value) -> Self {
            self.outputs.push((k, v));
            self
        }
        /// Mark the stub as a judging action (its contract would list
        /// a `satisfied` output) for implicit-judgment tests (#253).
        fn judging(mut self) -> Self {
            self.judges = true;
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
        fn judges(&self) -> bool {
            self.judges
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

    async fn engine_for_test() -> (Engine, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .expect("open test store");
        let mut e = Engine {
            registry: BTreeMap::new(),
            store: Some(Arc::new(store)),
            browser: None,
            definition_path: None,
            filter: None,
            seed: None,
            run_id: None,
            scope: RunScope::default(),
            skip_env_up: false,
            keep_env: false,
            env: BTreeMap::new(),
            inherited: std::collections::HashSet::new(),
            default_within: None,
            inconclusive_policy: InconclusivePolicy::Block,
            retry: None,
            // Zero backoff so retry-loop tests run instantly.
            retry_backoff_base: Duration::ZERO,
            capture: CapturePolicy::default(),
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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

    /// Read back the events of the (single) run the engine left in
    /// the test store.
    async fn read_only_run_events(engine: &Engine) -> Vec<duhem_evidence::Event> {
        let store = engine.store.as_ref().expect("test engine has a store");
        let runs = store.list_runs().await.unwrap();
        assert_eq!(runs.len(), 1, "exactly one run in the store");
        duhem_evidence::Trace::from_store(store.as_ref(), &runs[0].run_id)
            .await
            .unwrap()
            .into_events()
    }

    #[tokio::test]
    async fn missing_browser_for_page_step_yields_environment_error_not_pass() {
        let (mut engine, _tmp) = engine_for_test().await;
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
        let events = read_only_run_events(&engine).await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let events = read_only_run_events(&engine).await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let events = read_only_run_events(&engine).await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let (mut engine, _tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(StubAction::new("fake/boom", Outcome::Error)));
        let criterion_calls = Arc::new(AtomicUsize::new(0));
        let criterion_tracker = StubAction {
            uses: "fake/criterion",
            outcome: Outcome::Ok,
            outputs: Vec::new(),
            invocations: criterion_calls.clone(),
            judges: false,
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
        let events = read_only_run_events(&engine).await;
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
        let (mut engine, _tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(StubAction::new("fake/slow", Outcome::Timeout)));
        let criterion_calls = Arc::new(AtomicUsize::new(0));
        let criterion_tracker = StubAction {
            uses: "fake/criterion",
            outcome: Outcome::Ok,
            outputs: Vec::new(),
            invocations: criterion_calls.clone(),
            judges: false,
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
        let events = read_only_run_events(&engine).await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let events = read_only_run_events(&engine).await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let events = read_only_run_events(&engine).await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let events = read_only_run_events(&engine).await;
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let (mut e1, _tmp1) = engine_for_test().await;
        e1.seed = Some(42);
        let v1 = e1.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(
            v1.state,
            VerdictState::Pass,
            "seed=42 must evaluate $runtime.uuid() to the seeded literal"
        );

        // Sanity: a second seeded engine reaches the same verdict.
        let (mut e2, _tmp2) = engine_for_test().await;
        e2.seed = Some(42);
        let v2 = e2.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(v2.state, VerdictState::Pass);

        // Sanity: omitting the seed flips the verdict — `Uuid::new_v4`
        // colliding with the seeded literal would be a one-in-2^122
        // event, so this is a real determinism signal.
        let (mut e3, _tmp3) = engine_for_test().await;
        let v3 = e3.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(
            v3.state,
            VerdictState::Fail,
            "unseeded run should not accidentally match the seeded uuid"
        );
    }

    #[tokio::test]
    async fn run_verdict_preserves_document_order_of_criteria() {
        let (mut engine, _tmp) = engine_for_test().await;
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

        let (mut engine, tmp) = engine_for_test().await;
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

        let events = duhem_evidence::Trace::from_store(
            engine.store.as_ref().unwrap().as_ref(),
            &outcome.run_id,
        )
        .await
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

        let (mut engine, tmp) = engine_for_test().await;
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

        let events = duhem_evidence::Trace::from_store(
            engine.store.as_ref().unwrap().as_ref(),
            &outcome.run_id,
        )
        .await
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

        let (mut engine, tmp) = engine_for_test().await;
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

        let events = duhem_evidence::Trace::from_store(
            engine.store.as_ref().unwrap().as_ref(),
            &outcome.run_id,
        )
        .await
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

        let (mut engine, tmp) = engine_for_test().await;
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

        let events = duhem_evidence::Trace::from_store(
            engine.store.as_ref().unwrap().as_ref(),
            &outcome.run_id,
        )
        .await
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

        let (mut engine, tmp) = engine_for_test().await;
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

        let events = duhem_evidence::Trace::from_store(
            engine.store.as_ref().unwrap().as_ref(),
            &outcome.run_id,
        )
        .await
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
        let (mut engine, _tmp) = engine_for_test().await;
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
        let events = read_only_run_events(&engine).await;
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

    /// Spec #68: a seeded `$env` whitelist (`Engine::with_env`) is
    /// reachable from an assertion. The same definition with no seeded
    /// env leaves `$env.base_url` unwhitelisted, so the assertion is
    /// inconclusive rather than a pass — proving the whitelist, not a
    /// process-env leak, is what makes `$env` resolve.
    #[tokio::test]
    async fn seeded_env_whitelist_is_reachable_from_assertion() {
        let v = def(r#"
verification: env-reach
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - $env.base_url == "https://staging.example.com"
"#);
        // With the env seeded, the assertion passes.
        let (engine, _tmp) = engine_for_test().await;
        let mut env = BTreeMap::new();
        env.insert(
            "base_url".to_string(),
            "https://staging.example.com".to_string(),
        );
        let mut engine = engine.with_env(env);
        let outcome = engine.run_with_metadata(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(outcome.verdict.state, VerdictState::Pass);

        // With no env seeded, `$env.base_url` is not whitelisted: the
        // verdict is not a pass.
        let (mut bare, _tmp2) = engine_for_test().await;
        let bare_outcome = bare.run_with_metadata(&v, BTreeMap::new()).await.unwrap();
        assert_ne!(bare_outcome.verdict.state, VerdictState::Pass);
    }

    /// Spec #135: a leaf referencing an `inherits:` name that nothing
    /// bound fails LOUDLY with the specific remedy — not a generic deep
    /// failure — before any check runs.
    #[tokio::test]
    async fn unbound_inherited_input_errors_loudly() {
        let v = def(r#"
verification: leaf
inherits:
  - login_url
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.login_url == "x"
"#);
        let (engine, _tmp) = engine_for_test().await;
        let mut engine = engine.with_inherited(v.inherits.clone());
        let err = engine
            .run_with_metadata(&v, BTreeMap::new())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, EngineError::UnresolvedInheritedInput { name } if name == "login_url"),
            "got: {err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("login_url"), "{msg}");
        assert!(msg.contains("inherits:"), "{msg}");
        assert!(msg.contains("--inputs"), "{msg}");
    }

    /// Spec #135: when the inherited name IS bound (here via the
    /// resolved input map the CLI would have populated from the
    /// manifest's environment chain), the guard passes and the
    /// assertion sees the value.
    #[tokio::test]
    async fn bound_inherited_input_passes() {
        let v = def(r#"
verification: leaf
inherits:
  - login_url
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.login_url == "https://example.test/login"
"#);
        let (engine, _tmp) = engine_for_test().await;
        let mut engine = engine.with_inherited(v.inherits.clone());
        let mut inputs = BTreeMap::new();
        inputs.insert(
            "login_url".to_string(),
            serde_json::json!("https://example.test/login"),
        );
        let outcome = engine.run_with_metadata(&v, inputs).await.unwrap();
        assert_eq!(outcome.verdict.state, VerdictState::Pass);
    }

    /// Spec #135: an inherited name used only inside `$runtime.default`'s
    /// first argument is missing-tolerant — the guard must not fire.
    #[tokio::test]
    async fn inherited_under_default_carveout_does_not_trip_guard() {
        let v = def(r#"
verification: leaf
inherits:
  - maybe
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - $runtime.default($inputs.maybe, "x") == "x"
"#);
        let (engine, _tmp) = engine_for_test().await;
        let mut engine = engine.with_inherited(v.inherits.clone());
        let outcome = engine.run_with_metadata(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(outcome.verdict.state, VerdictState::Pass);
    }

    // -- manifest defaults: timeout / retry / inconclusive_policy (#66) ------

    /// Stub that records the `within` (integer ms) it was handed in its
    /// `with:` payload — `u64::MAX` when the payload omits it. Lets the
    /// timeout-threading tests observe what the engine injected.
    struct WithinCapture {
        uses: &'static str,
        seen: Arc<AtomicU64>,
    }

    #[async_trait]
    impl Dispatch for WithinCapture {
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
            with: &serde_yml::Value,
        ) -> Result<ActionResult, ActionError> {
            let ms = with
                .get(serde_yml::Value::String("within".to_string()))
                .and_then(|v| v.as_u64())
                .unwrap_or(u64::MAX);
            self.seen.store(ms, Ordering::SeqCst);
            Ok(ActionResult::ok())
        }
    }

    #[tokio::test]
    async fn default_timeout_fills_within_when_step_omits_it() {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.default_within = Some(Duration::from_secs(7));
        let seen = Arc::new(AtomicU64::new(0));
        engine.register_test_action(Box::new(WithinCapture {
            uses: "fake/cap",
            seen: seen.clone(),
        }));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/cap
            with: {}
        assertions:
          - "true"
"#);
        engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(
            seen.load(Ordering::SeqCst),
            7_000,
            "default timeout fills within"
        );
    }

    #[tokio::test]
    async fn per_step_within_wins_over_default_timeout() {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.default_within = Some(Duration::from_secs(7));
        let seen = Arc::new(AtomicU64::new(0));
        engine.register_test_action(Box::new(WithinCapture {
            uses: "fake/cap",
            seen: seen.clone(),
        }));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/cap
            with: { within: 2000 }
        assertions:
          - "true"
"#);
        engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(seen.load(Ordering::SeqCst), 2_000, "per-step within wins");
    }

    #[tokio::test]
    async fn retryable_inconclusive_retries_up_to_max() {
        // An Inconclusive(EnvironmentError) check (here via an invalid
        // regex pattern) re-runs from step 0 up to `max` times: 1
        // initial attempt + 2 retries = 3 step invocations.
        let (mut engine, _tmp) = engine_for_test().await;
        engine.retry = Some(RetryPolicy {
            max: 2,
            backoff: RetryBackoff::Exponential,
        });
        let stub = StubAction::new("fake/count", Outcome::Ok);
        let calls = stub.invocations.clone();
        engine.register_test_action(Box::new(stub));
        let v = def(r#"
verification: t
inputs:
  x: { type: string }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/count
        assertions:
          - matches: { value: $inputs.x, pattern: "[" }
"#);
        let mut inputs = BTreeMap::new();
        inputs.insert("x".to_string(), serde_json::Value::String("ok".to_string()));
        let verdict = engine.run(&v, inputs).await.unwrap();
        assert!(matches!(
            verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::EnvironmentError)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 3, "1 initial + 2 retries");
    }

    #[tokio::test]
    async fn fail_never_retries() {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.retry = Some(RetryPolicy {
            max: 3,
            backoff: RetryBackoff::Linear,
        });
        let stub = StubAction::new("fake/count", Outcome::Ok);
        let calls = stub.invocations.clone();
        engine.register_test_action(Box::new(stub));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/count
        assertions:
          - "false"
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(verdict.state, VerdictState::Fail);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "fail must not retry");
    }

    #[tokio::test]
    async fn missing_observation_inconclusive_not_retried() {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.retry = Some(RetryPolicy {
            max: 3,
            backoff: RetryBackoff::Exponential,
        });
        let stub = StubAction::new("fake/count", Outcome::Ok);
        let calls = stub.invocations.clone();
        engine.register_test_action(Box::new(stub));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - id: s1
            uses: fake/count
        assertions:
          - $steps.s1.outputs.missing == 1
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert!(matches!(
            verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation)
        ));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "missing_observation must not retry"
        );
    }

    const UNKNOWN_ACTION_VD: &str = r#"
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
"#;

    #[tokio::test]
    async fn inconclusive_policy_warn_softens_to_pass_with_warning() {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.inconclusive_policy = InconclusivePolicy::Warn;
        let v = def(UNKNOWN_ACTION_VD);
        let o = engine.run_with_metadata(&v, BTreeMap::new()).await.unwrap();
        // The criterion aggregated to Inconclusive but `warn` lifts it
        // to a criterion-level pass → the run passes, with a warning.
        assert_eq!(o.verdict.state, VerdictState::Pass);
        assert_eq!(o.verdict.criteria[0].state, VerdictState::Pass);
        assert_eq!(o.warnings.len(), 1, "one warning surfaced");
        assert!(o.warnings[0].contains("AC-1"), "{:?}", o.warnings);
    }

    #[tokio::test]
    async fn inconclusive_policy_pass_softens_silently() {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.inconclusive_policy = InconclusivePolicy::Pass;
        let v = def(UNKNOWN_ACTION_VD);
        let o = engine.run_with_metadata(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(o.verdict.state, VerdictState::Pass);
        assert!(o.warnings.is_empty(), "pass policy is silent");
    }

    #[tokio::test]
    async fn inconclusive_policy_block_preserves_todays_behavior() {
        let (mut engine, _tmp) = engine_for_test().await;
        // default policy is Block.
        let v = def(UNKNOWN_ACTION_VD);
        let o = engine.run_with_metadata(&v, BTreeMap::new()).await.unwrap();
        assert!(matches!(
            o.verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation)
        ));
        assert!(o.warnings.is_empty());
    }

    // ---- implicit judgment (spec #253) ----

    #[tokio::test]
    async fn implicit_satisfied_true_passes_without_assertions() {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(
            StubAction::new("fake/assert", Outcome::Ok)
                .with_output("satisfied", serde_json::json!(true))
                .judging(),
        ));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/assert
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(verdict.state, VerdictState::Pass);
    }

    #[tokio::test]
    async fn implicit_satisfied_false_fails_the_check() {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(
            StubAction::new("fake/assert", Outcome::Ok)
                .with_output("satisfied", serde_json::json!(false))
                .judging(),
        ));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/assert
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(verdict.state, VerdictState::Fail);
    }

    #[tokio::test]
    async fn binding_satisfied_takes_manual_control() {
        // Author binds `satisfied` (e.g. for a disjunction): the
        // implicit path is disabled, so a false observation judged
        // only by the author's own assertion still passes.
        let (mut engine, _tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(
            StubAction::new("fake/assert", Outcome::Ok)
                .with_output("satisfied", serde_json::json!(false))
                .judging(),
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
            uses: fake/assert
            outputs:
              satisfied: satisfied
        assertions:
          - $steps.s1.outputs.satisfied == false
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(verdict.state, VerdictState::Pass);
    }

    #[tokio::test]
    async fn implicit_is_inconclusive_when_step_was_skipped() {
        // An earlier step errors and aborts the check; the judging
        // step never runs, so its implicit assertion can't observe —
        // Inconclusive(MissingObservation), never a silent Pass.
        let (mut engine, _tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(StubAction::new("fake/error", Outcome::Error)));
        engine.register_test_action(Box::new(
            StubAction::new("fake/assert", Outcome::Ok)
                .with_output("satisfied", serde_json::json!(true))
                .judging(),
        ));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/error
          - uses: fake/assert
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert!(matches!(
            verdict.criteria[0].checks[0].state,
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation)
        ));
    }

    #[tokio::test]
    async fn explicit_and_implicit_assertions_coexist() {
        // An explicit assertion failing must fail the check even when
        // the implicit judgment passes — the fold sees both.
        let (mut engine, _tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(
            StubAction::new("fake/assert", Outcome::Ok)
                .with_output("satisfied", serde_json::json!(true))
                .judging(),
        ));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/assert
        assertions:
          - "false"
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert_eq!(verdict.state, VerdictState::Fail);
    }

    #[tokio::test]
    async fn no_assertions_and_no_judging_step_is_inconclusive_empty() {
        // Defensive path: the CLI's contract layer rejects this at
        // validate time; if it reaches the engine anyway, the empty
        // fold surfaces as Inconclusive rather than a silent Pass.
        let (mut engine, _tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(StubAction::new("fake/noop", Outcome::Ok)));
        let v = def(r#"
verification: t
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: fake/noop
"#);
        let verdict = engine.run(&v, BTreeMap::new()).await.unwrap();
        assert!(matches!(
            verdict.criteria[0].checks[0].state,
            VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation)
        ));
    }
}
