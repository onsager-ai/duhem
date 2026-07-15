//! Engine-facing outcome + error types (split from `runner.rs` for
//! the file-token budget).

use duhem_actions::ActionError;
use duhem_evidence::{StoreError, VerdictState, WriterError};
use duhem_judge::RunVerdict;
use thiserror::Error;

/// Surfaces only "the runtime itself failed" cases. A failing
/// artifact yields `RunVerdict::Fail`, not `Err`.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Evidence could not be written to the store.
    #[error("evidence: {0}")]
    Evidence(#[from] WriterError),
    /// The evidence store itself could not be opened or resolved.
    #[error("store: {0}")]
    Store(#[from] StoreError),
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
    /// A `$...` reference inside a step's `with:` payload resolved to
    /// nothing at runtime. We refuse to hand an action a literal
    /// `$...` string (#134): the reference is either undeclared or its
    /// upstream value is absent. `reference` is pinpointed to the
    /// smallest offending sub-expression — for a `$runtime.format(...)`
    /// argument that didn't resolve, it names the *argument*, not the
    /// whole call, and `context` carries the enclosing expression so the
    /// failure isn't misattributed to the function (#238). Names the
    /// step too, so the failure points at the VD, not a phantom SUT.
    #[error("step `{step}`: unresolved reference `{reference}` in `with:`{context}")]
    UnresolvedReference {
        reference: String,
        step: String,
        /// Preformatted enclosing-expression suffix — either empty (the
        /// reference is the whole `with:` value) or ` (evaluating
        /// `<expr>`)` when the reference is a sub-part of a larger call.
        context: String,
    },
    /// A `$inputs.<name>` reference names an input the leaf declared
    /// under `inherits:` (spec #135), but nothing on the resolution
    /// chain bound it — no manifest environment was selected and no
    /// `--inputs` supplied it. Distinct from the generic
    /// `UnresolvedReference` so the remedy (run the suite, or pass
    /// `--inputs`) is named instead of a deep network failure.
    #[error(
        "input `{name}` is declared `inherits:` but no environment or --inputs provides it; run the suite (e.g. `duhem run verifications/<suite>`) or pass `--inputs {name}=...`"
    )]
    UnresolvedInheritedInput { name: String },
}

impl From<ActionError> for EngineError {
    fn from(e: ActionError) -> Self {
        EngineError::Browser(e.to_string())
    }
}

/// A step's diagnostic label: its `id` when declared, else
/// `<uses> #<index>`. Used to name a step in an
/// [`EngineError::UnresolvedReference`] so the author can locate the
/// offending `with:` reference even in an anonymous step.
pub(crate) fn step_label(step: &duhem_schema::Step, idx: usize) -> String {
    step.id
        .clone()
        .unwrap_or_else(|| format!("{} #{idx}", step.uses))
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

/// Engine-side run summary that carries the run identity alongside
/// the verdict. Returned by [`Engine::run_with_metadata`]; thin
/// convenience around [`Engine::run`] for callers (the CLI's
/// `--reporter json`, replay tooling) that read the run back from the
/// store by `run_id`.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub verdict: RunVerdict,
    pub run_id: String,
    /// Checks that did not pass, each with its failing assertions.
    /// Carried out of the run so reporters can show *which* assertion
    /// failed (and any cause detail) without the author trace-reading
    /// the store. Empty on a fully-passing run.
    pub failures: Vec<CheckFailure>,
    /// Non-fatal warnings produced during the run — currently the
    /// `inconclusive_policy: warn` notices (spec #66): a criterion that
    /// aggregated to `inconclusive` but was treated as a pass by the
    /// manifest default. Empty unless `warn` softened something. The
    /// reporter surfaces these in the run summary.
    pub warnings: Vec<String>,
}

/// One assertion that failed or was inconclusive within a check.
#[derive(Debug, Clone)]
pub struct FailedAssertion {
    /// The human-authored assertion line, reconstructed from the schema
    /// (`assertion_to_expr`) — e.g. `$steps.q.outputs.status == 200`.
    pub expr: String,
    /// `Fail` or `Inconclusive(..)`; never `Pass` (passing assertions
    /// are not collected).
    pub state: VerdictState,
    /// Evidence-bound cause detail when present (e.g. a missing
    /// observation or type mismatch). `None` for a plain comparison
    /// that evaluated false — the expression itself localizes it.
    pub detail: Option<String>,
}

/// One non-passing check and the assertions that explain its verdict.
#[derive(Debug, Clone)]
pub struct CheckFailure {
    pub criterion_id: String,
    pub check_id: String,
    pub assertions: Vec<FailedAssertion>,
    /// Failure-evidence captures recorded for this check (spec #202):
    /// the `capture/*` blob observations, so the reporter can point
    /// at the picture without trace-reading. Empty when capture is
    /// off, the check had no browser, or every capture op failed.
    pub captures: Vec<CapturedArtifact>,
}

/// One runner-emitted capture: the reserved `capture/*` output name
/// and the content address of its blob in the evidence store.
#[derive(Debug, Clone)]
pub struct CapturedArtifact {
    pub kind: String,
    pub sha256: String,
}
