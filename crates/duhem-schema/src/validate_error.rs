//! Error types emitted by structural Verification Definition validation.
//!
//! Split out of `validate.rs` (#411), which was over the per-file token
//! budget and passing CI only on a standing `// budget-allow:`
//! exemption. The error vocabulary is self-contained and changes for
//! different reasons than the validation walks, so it is the natural
//! seam.

use thiserror::Error;

use crate::SourceLocation;
use crate::verification::InputType;

/// Where a `$...` reference was authored. Renders into a
/// [`ValidationError`] message so a `with:` ref names its step rather
/// than masquerading as an assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefSite {
    /// Inside a check's `assertions:` list.
    Assertion,
    /// The check-level browser-session seed expression.
    Session,
    /// Inside a step's `with:` payload. Carries the step's label —
    /// its `id` when declared, else `step <index>`.
    StepWith { step: String },
}

impl std::fmt::Display for RefSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefSite::Assertion => write!(f, "assertion"),
            RefSite::Session => write!(f, "session:"),
            RefSite::StepWith { step } => write!(f, "step `{step}` with:"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("verification has no criteria")]
    NoCriteria,

    #[error("{site} `{raw}` is not a valid expression: {context}")]
    InvalidWithExpression {
        raw: String,
        context: String,
        site: RefSite,
        location: Option<SourceLocation>,
    },

    #[error("duplicate criterion id `{id}`")]
    DuplicateCriterionId { id: String },

    #[error("criterion `{criterion}`: duplicate check id `{id}`")]
    DuplicateCheckId { criterion: String, id: String },

    #[error(
        "criterion `{criterion}` / check `{check}`: nothing to judge — no assertions and no steps (spec #253: a check may omit `assertions:` only when a judging step carries the verdict)"
    )]
    NothingToJudge { criterion: String, check: String },

    #[error("criterion `{criterion}` / check `{check}`: duplicate step id `{id}`")]
    DuplicateStepId {
        criterion: String,
        check: String,
        id: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: step `{step}` output `{name}` uses the reserved `capture/` prefix (runner-emitted failure evidence, spec #202)"
    )]
    ReservedOutputPrefix {
        criterion: String,
        check: String,
        step: String,
        name: String,
    },

    #[error(
        "{site}: step `{step}` secret output path `{path}` names undeclared action output `{output}`"
    )]
    UndeclaredSecretOutput {
        site: String,
        step: String,
        path: String,
        output: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references undeclared step `{step}`"
    )]
    UnresolvedStepRef {
        criterion: String,
        check: String,
        step: String,
        raw: String,
        site: RefSite,
        location: Option<SourceLocation>,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references undeclared output `{output}` on step `{step}`"
    )]
    UnresolvedStepOutput {
        criterion: String,
        check: String,
        step: String,
        output: String,
        raw: String,
        site: RefSite,
        location: Option<SourceLocation>,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references undeclared input `{input}`{help}"
    )]
    UnresolvedInputRef {
        criterion: String,
        check: String,
        input: String,
        raw: String,
        site: RefSite,
        help: String,
        location: Option<SourceLocation>,
    },

    #[error("{site} `{raw}` references unknown page locator `{entry}`{help}")]
    UnresolvedPageRef {
        site: String,
        raw: String,
        entry: String,
        help: String,
        location: Option<SourceLocation>,
    },

    #[error("page locator `{entry}` has invalid placeholder escaping: {detail}")]
    InvalidPageTemplate { entry: String, detail: String },

    #[error(
        "{site} `{raw}` supplies {given} {argument_word} to page locator `{entry}`, but the entry contains {placeholders} `{{}}` {placeholder_word}"
    )]
    PageRefArityMismatch {
        site: String,
        raw: String,
        entry: String,
        given: usize,
        argument_word: &'static str,
        placeholders: usize,
        placeholder_word: &'static str,
        location: Option<SourceLocation>,
    },

    #[error("{site} `{raw}` references unknown `$runtime` helper `{helper}`{help}")]
    UnknownRuntimeHelper {
        site: String,
        raw: String,
        helper: String,
        help: String,
    },

    #[error("{site} `{raw}` references `$runtime.{helper}` without calling it; use `{call_form}`")]
    UncalledRuntimeHelper {
        site: String,
        raw: String,
        helper: String,
        call_form: String,
    },

    #[error(
        "{site} `{raw}` calls `$runtime.{helper}` with {given} {argument_word}; expected {expected}"
    )]
    WrongRuntimeHelperArity {
        site: String,
        raw: String,
        helper: String,
        given: usize,
        argument_word: &'static str,
        expected: String,
    },

    #[error("{site} `{raw}`: malformed `$pages` reference (expected `$pages.<page>.<element>`)")]
    MalformedPageRef {
        site: String,
        raw: String,
        location: Option<SourceLocation>,
    },

    #[error(
        "page locator `$pages.{page}.{element}` references input `{input}`, but leaf `{verification}` does not declare it"
    )]
    PageInputUndeclared {
        verification: String,
        page: String,
        element: String,
        input: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}`: malformed `$steps` reference (expected `$steps.<step_id>.outputs.<output>`)"
    )]
    MalformedStepRef {
        criterion: String,
        check: String,
        raw: String,
        site: RefSite,
        location: Option<SourceLocation>,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}`: malformed `$inputs` reference (expected `$inputs.<name>`)"
    )]
    MalformedInputRef {
        criterion: String,
        check: String,
        raw: String,
        site: RefSite,
        location: Option<SourceLocation>,
    },

    #[error("setup: duplicate step id `{id}`")]
    DuplicateSetupStepId { id: String },

    #[error("teardown: duplicate step id `{id}`")]
    DuplicateTeardownStepId { id: String },

    #[error("{label} {phase}: duplicate step id `{id}`")]
    DuplicateLifecycleStepId {
        label: String,
        phase: &'static str,
        id: String,
        location: Option<SourceLocation>,
    },

    #[error(
        "{label} {phase} step id `{id}` collides with an outer lifecycle step id already in scope — step ids must be unique across nested `setup:`/`teardown:` scopes (#441)"
    )]
    LifecycleStepIdCollision {
        label: String,
        phase: &'static str,
        id: String,
        location: Option<SourceLocation>,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references undeclared setup step `{step}`"
    )]
    UnresolvedSetupStepRef {
        criterion: String,
        check: String,
        step: String,
        raw: String,
        site: RefSite,
        location: Option<SourceLocation>,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references setup step `{step}`, which is declared but out of scope: a criterion- or check-level `setup:` step's outputs are not addressable from the check body (only leaf-level `setup:` is). If `{step}` should produce a value this check reads, move it to the leaf-level `setup:` block instead; `fixtures:` + `needs:` (#449) is a different tool — guaranteed per-check cleanup that can address what it created, but only from that fixture's own `down:`, never from a check body either"
    )]
    SetupStepOutOfScope {
        criterion: String,
        check: String,
        step: String,
        raw: String,
        site: RefSite,
        location: Option<SourceLocation>,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references undeclared output `{output}` on setup step `{step}`"
    )]
    UnresolvedSetupStepOutput {
        criterion: String,
        check: String,
        step: String,
        output: String,
        raw: String,
        site: RefSite,
        location: Option<SourceLocation>,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}`: malformed `$setup` reference (expected `$setup.<step_id>.outputs.<output>`)"
    )]
    MalformedSetupRef {
        criterion: String,
        check: String,
        raw: String,
        site: RefSite,
        location: Option<SourceLocation>,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: session `{value}` must be a whole-string `$` reference (for example `$setup.login.outputs.state` or `$inputs.session_state`)"
    )]
    InvalidSessionReference {
        criterion: String,
        check: String,
        value: String,
        location: Option<SourceLocation>,
    },

    #[error("{message}")]
    InvalidNamedSession {
        message: String,
        location: Option<SourceLocation>,
    },

    #[error(
        "input `{input}`: default value type `{actual}` does not match declared type `{declared}`"
    )]
    InputDefaultTypeMismatch {
        input: String,
        declared: InputType,
        actual: String,
    },

    #[error(
        "input `{input}`: `secret: true` cannot be combined with `default:` — supply the value via `env:`, a selected profile, or `--inputs`"
    )]
    SecretInputHasDefault { input: String },

    #[error("input `{input}`: env name `{name}` is invalid — expected `[A-Z_][A-Z0-9_]*`")]
    InvalidInputEnvName { input: String, name: String },

    #[error("input `{input}`: `type:` is required unless `inherit: true`")]
    InputMissingType { input: String },

    #[error(
        "input `{input}`: `inherit: true` cannot be combined with `type:` — declare the type under manifest `inputs.{input}`"
    )]
    InheritedInputHasType { input: String },

    #[error(
        "input `{input}`: `inherit: true` cannot be combined with `default:` — declare the default under manifest `inputs.{input}`"
    )]
    InheritedInputHasDefault { input: String },

    #[error(
        "manifest input `{input}` cannot set `inherit: true` — inheritance is declared by a consuming leaf"
    )]
    ManifestInputCannotInherit { input: String },

    #[error("{0}")]
    BadProjectDecl(String),

    #[error("{message}")]
    InvalidFlow { message: String },

    #[error("unknown action `{uses}` (see `duhem actions` for the registered catalog)")]
    UnknownAction {
        uses: String,
        location: Option<SourceLocation>,
    },

    #[error(
        "criterion `{criterion}` / check `{check}` has no computable worst-case step count: {reason}"
    )]
    UncomputableStepCount {
        criterion: String,
        check: String,
        reason: String,
    },

    #[error("{message}")]
    InvalidStepCondition {
        message: String,
        location: Option<SourceLocation>,
    },

    #[error(
        "{site} references `${name}`, but `${name}` is not a `for_each` binding in scope here (expected `$steps`, `$setup`, `$fixture`, `$inputs`, `$pages`, `$env`, `$runtime`, or the `as:` name of an enclosing `for_each`)"
    )]
    LoopVariableOutOfScope {
        site: String,
        name: String,
        location: Option<SourceLocation>,
    },

    #[error(
        "{site}: `for_each:` requires `max:` — the iteration ceiling #444 needs to compute a worst-case step count"
    )]
    ForEachMaxRequired {
        site: String,
        location: Option<SourceLocation>,
    },

    #[error(
        "{site}: a `for_each:` body step may not itself declare `{field}:` — nesting is capped at depth 1"
    )]
    ForEachDepthExceeded {
        site: String,
        field: &'static str,
        location: Option<SourceLocation>,
    },

    #[error(
        "{site}: `{field}:` is not allowed on a `for_each:` step — it produces no output of its own; give the *body* step an `id:` instead"
    )]
    ForEachWrapperFieldNotAllowed {
        site: String,
        field: &'static str,
        location: Option<SourceLocation>,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: value-based conditions are not yet available on check steps"
    )]
    CheckStepConditionUnavailable {
        criterion: String,
        check: String,
        location: Option<SourceLocation>,
    },

    #[error("fixture `{fixture}` must declare at least one `{phase}:` step")]
    EmptyFixturePhase {
        fixture: String,
        phase: &'static str,
    },

    #[error("criterion `{criterion}` / check `{check}` needs undeclared fixture `{fixture}`")]
    UndeclaredFixture {
        criterion: String,
        check: String,
        fixture: String,
        location: Option<SourceLocation>,
    },

    #[error("{site}: fixture references are only valid in that fixture's own `down:` block")]
    FixtureRefOutsideDown {
        site: String,
        location: Option<SourceLocation>,
    },

    #[error("fixture `{fixture}` {phase} step `{step}` may not carry `needs:`")]
    FixtureStepNeeds {
        fixture: String,
        phase: &'static str,
        step: String,
        location: Option<SourceLocation>,
    },

    #[error(
        "fixture `{fixture}` down step `{down_step}` references invalid fixture output `{raw}`"
    )]
    InvalidFixtureRef {
        fixture: String,
        down_step: String,
        raw: String,
        location: Option<SourceLocation>,
    },
}

impl ValidationError {
    /// Exact source position of the offending scalar, when proven.
    pub fn location(&self) -> Option<SourceLocation> {
        match self {
            Self::InvalidWithExpression { location, .. }
            | Self::UnresolvedStepRef { location, .. }
            | Self::UnresolvedStepOutput { location, .. }
            | Self::UnresolvedInputRef { location, .. }
            | Self::UnresolvedPageRef { location, .. }
            | Self::PageRefArityMismatch { location, .. }
            | Self::MalformedPageRef { location, .. }
            | Self::MalformedStepRef { location, .. }
            | Self::MalformedInputRef { location, .. }
            | Self::UnresolvedSetupStepRef { location, .. }
            | Self::UnresolvedSetupStepOutput { location, .. }
            | Self::SetupStepOutOfScope { location, .. }
            | Self::MalformedSetupRef { location, .. }
            | Self::InvalidSessionReference { location, .. }
            | Self::InvalidNamedSession { location, .. }
            | Self::UnknownAction { location, .. }
            | Self::InvalidStepCondition { location, .. }
            | Self::CheckStepConditionUnavailable { location, .. }
            | Self::LoopVariableOutOfScope { location, .. }
            | Self::ForEachMaxRequired { location, .. }
            | Self::ForEachDepthExceeded { location, .. }
            | Self::ForEachWrapperFieldNotAllowed { location, .. } => *location,
            Self::UndeclaredFixture { location, .. }
            | Self::FixtureRefOutsideDown { location, .. }
            | Self::FixtureStepNeeds { location, .. }
            | Self::InvalidFixtureRef { location, .. } => *location,
            Self::DuplicateLifecycleStepId { location, .. }
            | Self::LifecycleStepIdCollision { location, .. } => *location,
            _ => None,
        }
    }
}
