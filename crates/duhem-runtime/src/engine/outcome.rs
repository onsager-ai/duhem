//! Engine-facing outcome + error types (split from `runner.rs` for
//! the file-token budget).

use std::collections::{BTreeMap, BTreeSet};

use duhem_actions::{ActionError, ExistenceState, Locator};
use duhem_evidence::{EventPayload, EvidenceWriter, StoreError, WriterError};
use duhem_judge::{AssertionOutcome, InconclusiveCause, RunVerdict, VerdictState};
use duhem_schema::{Expr, PathRoot};
use thiserror::Error;

use crate::engine::context::RunContext;
use crate::engine::shim::assertion_to_expr;
use crate::engine::translate::{eval_cause_detail, eval_to_state};
use crate::eval::{EvalResult, describe_comparison, eval};

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
    /// The runtime trapped an operator termination signal after
    /// recording an append-only `run_aborted` marker.
    #[error("run aborted by {signal}")]
    Aborted { signal: String },
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
    /// with `inherit: true` (spec #135), but nothing on the resolution
    /// chain bound it — no manifest profile was selected and no
    /// `--inputs` supplied it. Distinct from the generic
    /// `UnresolvedReference` so the remedy (run the suite, or pass
    /// `--inputs`) is named instead of a deep network failure.
    #[error(
        "input `{name}` is declared `inherit: true` but no profile or --inputs provides it; run the suite (e.g. `duhem run verifications/<suite>`) or pass `--inputs {name}=...`"
    )]
    UnresolvedInheritedInput { name: String },
    /// A validated secret path started at a declared output, but the
    /// action did not return that value on this invocation.
    #[error("step `{step}`: secret output path `{path}` did not resolve to a value")]
    SecretOutputMissing { step: String, path: String },
    /// Exact-substring registration protects one scalar spelling.
    /// Registering an aggregate's serialization would look protected
    /// while missing the values that actually recur in evidence.
    #[error(
        "secret_outputs: `{path}` resolved to an {shape}, not a value.\nName the scalar that is sensitive, e.g. `{path}.data`.\nRegistering a whole {shape} would mask only its exact\nserialization, which is almost never what appears in evidence."
    )]
    SecretOutputNotScalar { path: String, shape: &'static str },
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
/// What the runner observed at one step, retained so the implicit
/// judgment can speak the *reason* a step's verdict came out the way it
/// did — the authored intent (resolved `with:`, carrying the locator /
/// expectation / deadline) plus the action's recorded outputs (e.g.
/// `ui/assert-element`'s `count`). Empty (`with: Null`, no outputs) for
/// a step that never ran.
#[derive(Clone)]
pub(crate) struct StepEvidence {
    /// The step's resolved `with:` payload — references substituted, as
    /// the action actually saw it.
    pub with: serde_yml::Value,
    /// The action's outputs (`satisfied`, `count`, …) for this step.
    pub outputs: BTreeMap<String, serde_json::Value>,
    /// Why this step was gated out. A skipped step contributes no
    /// assertion verdict; this distinguishes it from an unrun step
    /// whose observation is genuinely missing.
    pub skip_reason: Option<String>,
    /// Authored catalog coordinate, retained after the map is spliced
    /// so a failure points back to the one shared entry to edit.
    pub catalog_reference: Option<String>,
}

impl StepEvidence {
    /// An empty record for a step that produced no observation (didn't
    /// run, or errored before returning outputs).
    pub fn empty() -> Self {
        StepEvidence {
            with: serde_yml::Value::Null,
            outputs: BTreeMap::new(),
            skip_reason: None,
            catalog_reference: None,
        }
    }

    pub fn skipped(reason: String) -> Self {
        StepEvidence {
            with: serde_yml::Value::Null,
            outputs: BTreeMap::new(),
            skip_reason: Some(reason),
            catalog_reference: None,
        }
    }

    /// The step's `satisfied` output as a bool, when it recorded one.
    fn satisfied(&self) -> Option<bool> {
        self.outputs.get("satisfied").and_then(|v| v.as_bool())
    }
}

/// One synthesized implicit-judgment outcome (spec #253) for a judging
/// step, ready for the runner to emit as an `AssertionEvaluated` event.
pub(crate) struct ImplicitOutcome {
    pub label: String,
    /// The 0-based index of the step this judgment is derived from, so
    /// the emitted `AssertionEvaluated` carries the step link (a
    /// reporter folds the assertion into its step and propagates its
    /// status).
    pub step_index: usize,
    pub state: VerdictState,
    pub detail: Option<String>,
}

/// Compute one step's implicit judgment, if the step contributes one.
/// This is also the single source of truth for gating: binding an output
/// named `satisfied` suppresses both the implicit assertion and any gate
/// derived from that assertion.
#[allow(clippy::too_many_arguments)]
pub(crate) fn implicit_judgment_for_step(
    step: &duhem_schema::Step,
    step_index: usize,
    action_judges: bool,
    evidence: &StepEvidence,
    any_unknown: bool,
    environment_failed: bool,
    browser_missing: bool,
) -> Option<ImplicitOutcome> {
    if !action_judges || step.outputs.contains_key("satisfied") || evidence.skip_reason.is_some() {
        return None;
    }

    let label = display_step_label(step, step_index);
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
        match evidence.satisfied() {
            Some(true) => (VerdictState::Pass, None),
            Some(false) => (
                VerdictState::Fail,
                Some(judging_fail_detail(step, evidence, &label)),
            ),
            None => (
                VerdictState::Inconclusive(InconclusiveCause::MissingObservation),
                Some(format!(
                    "step `{label}` did not run or produced no `satisfied`"
                )),
            ),
        }
    };

    Some(ImplicitOutcome {
        label,
        step_index,
        state,
        detail,
    })
}

/// Compute the implicit assertion outcomes for a check's judging steps
/// (spec #253): one entry per step whose action judges (its contract
/// emits `satisfied`, tested via `is_judging`) and that hasn't bound
/// `satisfied` in its `outputs:` (binding it is the manual-control
/// opt-out). Cause mapping mirrors the explicit-assertion path — an
/// unknown action / environment failure / unrun step never yields a
/// silent `pass`. Kept out of `run_check` for the file-token budget.
pub(crate) fn implicit_judgment_outcomes(
    check: &duhem_schema::Check,
    is_judging: impl Fn(&str) -> bool,
    step_evidence: &[StepEvidence],
    any_unknown: bool,
    environment_failed: bool,
    browser_missing: bool,
) -> Vec<ImplicitOutcome> {
    check
        .steps
        .iter()
        .enumerate()
        .filter_map(|(idx, step)| {
            implicit_judgment_for_step(
                step,
                idx,
                is_judging(step.uses_name()),
                &step_evidence[idx],
                any_unknown,
                environment_failed,
                browser_missing,
            )
        })
        .collect()
}

/// A human, semantic failure detail for a judging step whose implicit
/// `satisfied` came back false — the counterpart to
/// `describe_comparison` on the explicit-assertion path. Names the
/// authored intent (locator / expectation / deadline) and what was
/// observed, so a reporter shows *why* rather than the opaque
/// `actual false, expected true`. Falls back to that plain form when
/// the step's `with:` can't be read (a `$`-ref that never resolved, an
/// action we don't specially humanize).
fn judging_fail_detail(step: &duhem_schema::Step, ev: &StepEvidence, label: &str) -> String {
    let specialized = match step.uses_name() {
        "ui/assert-element" => assert_element_fail_detail(ev),
        _ => intent_fail_detail(step, ev),
    };
    specialized.unwrap_or_else(|| generic_fail_detail(label))
}

/// The pre-#280 wording, kept as the last-resort fallback.
fn generic_fail_detail(label: &str) -> String {
    format!("actual false, expected true (implicit `satisfied` of step `{label}`)")
}

/// Optional ` within 5s` suffix, read from the step's resolved `with:`.
fn timeout_suffix(ev: &StepEvidence) -> String {
    ev.with
        .get("timeout")
        .and_then(|v| v.as_str())
        .map(|s| format!(" within {s}"))
        .unwrap_or_default()
}

/// `ui/assert-element`: "expected text \"Manager\" to be absent within
/// 5s, but 1 still matched". Reads the locator / `expected` / `timeout`
/// from the resolved `with:` and the observed `count` from the outputs.
fn assert_element_fail_detail(ev: &StepEvidence) -> Option<String> {
    let loc: Locator = serde_yml::from_value(ev.with.get("locator")?.clone()).ok()?;
    let expected: ExistenceState = serde_yml::from_value(ev.with.get("expected")?.clone()).ok()?;
    let resolved = loc.describe();
    let desc = ev
        .catalog_reference
        .as_ref()
        .map(|reference| format!("`{reference}` ({resolved})"))
        .unwrap_or(resolved);
    let timeout_clause = timeout_suffix(ev);
    let count = ev.outputs.get("count").and_then(|v| v.as_u64());
    Some(match expected {
        ExistenceState::NotExists => match count {
            Some(n) => {
                format!("expected {desc} to be absent{timeout_clause}, but {n} still matched")
            }
            None => format!("expected {desc} to be absent{timeout_clause}, but it was present"),
        },
        ExistenceState::Hidden => match count {
            Some(n) => {
                format!(
                    "expected {desc} to be hidden{timeout_clause}, but it stayed visible ({n} present)"
                )
            }
            None => format!("expected {desc} to be hidden{timeout_clause}, but it stayed visible"),
        },
        ExistenceState::Exists => {
            format!("expected {desc} to appear{timeout_clause}, but none was found")
        }
        ExistenceState::Visible => match count {
            Some(n) if n > 0 => {
                format!(
                    "expected {desc} to be visible{timeout_clause}, but it stayed hidden ({n} present)"
                )
            }
            _ => format!("expected {desc} to be visible{timeout_clause}, but it never appeared"),
        },
    })
}

/// Generic humanizer for the other judging actions (`ui/assert-url`,
/// `ui/assert-state`, `api/poll`, …): compose a short "what was
/// expected" line from the well-known `with:` fields, plus the last
/// observed HTTP status when the action recorded one. `None` when
/// there's nothing legible to say — the caller falls back to the plain
/// form.
fn intent_fail_detail(step: &duhem_schema::Step, ev: &StepEvidence) -> Option<String> {
    let s = |k: &str| ev.with.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let mut intent: Vec<String> = Vec::new();
    if let Some(m) = s("method") {
        intent.push(m);
    }
    if let Some(u) = s("url") {
        intent.push(u);
    }
    if let Some(exp) = s("expected") {
        intent.push(exp);
    }
    if let Some(until) = s("until") {
        intent.push(format!("until {until}"));
    }
    if let Some(eq) = s("equals") {
        intent.push(format!("equals \"{eq}\""));
    }
    if let Some(re) = s("matches") {
        intent.push(format!("matches /{re}/"));
    }
    if intent.is_empty() {
        return None;
    }
    let timeout_clause = timeout_suffix(ev);
    let observed = ev
        .outputs
        .get("status")
        .and_then(|v| v.as_u64())
        .map(|st| format!(" — last status {st}"))
        .unwrap_or_default();
    Some(format!(
        "`{}`: expected {}{timeout_clause}, but it did not hold{observed}",
        step.uses_name(),
        intent.join(" ")
    ))
}

/// Evaluate a check's explicit `assertions:` into outcomes + evidence
/// (indices `0..assertions.len()`), folding them into the running
/// `assertion_outcomes` / `failed` collections. An unknown action or
/// environment failure overrides every assertion to `inconclusive`
/// with the matching cause (the same prefix `implicit_judgment_outcomes`
/// applies), so the two evaluation paths agree. Split out of
/// `run_check` for the file-token budget.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn evaluate_explicit_assertions(
    writer: &mut EvidenceWriter,
    check: &duhem_schema::Check,
    ctx: &RunContext<'_>,
    any_unknown: bool,
    environment_failed: bool,
    browser_missing: bool,
    step_evidence: &[StepEvidence],
    assertion_outcomes: &mut Vec<AssertionOutcome>,
    failed: &mut Vec<FailedAssertion>,
) -> Result<(), EngineError> {
    for (i, assertion) in check.assertions.iter().enumerate() {
        let expr = assertion_to_expr(assertion);
        // The step this assertion is *about*, when it references exactly
        // one — so the reporter folds it onto that step and paints it
        // (an explicit `$steps.update.outputs.status == 200` IS about the
        // `update` step, #279 follow-up).
        let step_index = owning_step_index(&expr, check);
        if skipped_reference_reason(&expr, check, step_evidence).is_some() {
            continue;
        }
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
            let r = eval(&expr, ctx);
            let detail = match &r {
                EvalResult::Inconclusive(c) => Some(eval_cause_detail(c)),
                // A failed comparison gets its observed operands —
                // "actual <lhs>, expected <rhs>" — so the reporter
                // shows the values, not just the expression.
                EvalResult::False => describe_comparison(&expr, ctx),
                EvalResult::True => None,
            };
            (eval_to_state(&r), detail)
        };
        writer
            .append(EventPayload::AssertionEvaluated {
                check_id: check.id.clone(),
                assertion_index: i as u32,
                state,
                detail: detail.clone(),
                // Folds onto its step when the assertion references
                // exactly one; else standalone (zero/many steps).
                step_index,
                // Carry the authored line so the reporter shows *what*
                // was asserted, not just the observed values.
                expr: Some(assertion.display()),
            })
            .await?;
        if !matches!(state, VerdictState::Pass) {
            failed.push(FailedAssertion {
                expr: assertion.display(),
                state,
                detail: detail.clone(),
            });
        }
        assertion_outcomes.push(AssertionOutcome {
            assertion_index: i,
            state,
            detail,
        });
    }
    Ok(())
}

/// Collect the distinct `$steps.<id>` step ids referenced anywhere in an
/// assertion expression (walks operands, call args, and both sides of a
/// comparison).
fn steps_referenced(expr: &Expr, out: &mut BTreeSet<String>) {
    match expr {
        Expr::Path(p) => {
            if matches!(p.root, PathRoot::Steps)
                && let Some(id) = p.segments.first()
            {
                out.insert(id.clone());
            }
        }
        Expr::Call { path, args } => {
            if matches!(path.root, PathRoot::Steps)
                && let Some(id) = path.segments.first()
            {
                out.insert(id.clone());
            }
            for a in args {
                steps_referenced(a, out);
            }
        }
        Expr::BinOp { lhs, rhs, .. } => {
            steps_referenced(lhs, out);
            steps_referenced(rhs, out);
        }
        Expr::UnaryOp { expr, .. } => steps_referenced(expr, out),
        Expr::Lit(_) => {}
    }
}

fn skipped_reference_reason(
    expr: &Expr,
    check: &duhem_schema::Check,
    evidence: &[StepEvidence],
) -> Option<String> {
    let mut refs = BTreeSet::new();
    steps_referenced(expr, &mut refs);
    check.steps.iter().enumerate().find_map(|(index, step)| {
        let id = step.id.as_ref()?;
        if !refs.contains(id) {
            return None;
        }
        evidence.get(index)?.skip_reason.clone()
    })
}

/// If an explicit assertion references exactly one step (`$steps.<id>`),
/// return that step's 0-based index in the check — so the reporter folds
/// the assertion onto its step and propagates status (an assertion on a
/// single step's output IS about that step). Assertions that touch zero
/// or many steps (a literal comparison, a cross-step comparison) return
/// `None` and stay standalone. A reference to an `id`-less step (no
/// `$steps.<id>` can name it) also yields `None`.
fn owning_step_index(expr: &Expr, check: &duhem_schema::Check) -> Option<u32> {
    let mut refs = BTreeSet::new();
    steps_referenced(expr, &mut refs);
    if refs.len() != 1 {
        return None;
    }
    let only = refs.iter().next()?;
    check
        .steps
        .iter()
        .position(|s| s.id.as_deref() == Some(only.as_str()))
        .map(|i| i as u32)
}

/// Emit the implicit-judgment outcomes as `AssertionEvaluated` events
/// (indices continuing from `start_index`) and fold them into the
/// check's running `assertion_outcomes` / `failed` collections (spec
/// #253). Kept beside `implicit_judgment_outcomes` and out of
/// `run_check` for the file-token budget.
pub(crate) async fn append_implicit_judgment(
    writer: &mut EvidenceWriter,
    check_id: &str,
    outcomes: Vec<ImplicitOutcome>,
    start_index: usize,
    assertion_outcomes: &mut Vec<AssertionOutcome>,
    failed: &mut Vec<FailedAssertion>,
) -> Result<(), EngineError> {
    for (offset, imp) in outcomes.into_iter().enumerate() {
        let index = start_index + offset;
        writer
            .append(EventPayload::AssertionEvaluated {
                check_id: check_id.to_string(),
                assertion_index: index as u32,
                state: imp.state,
                detail: imp.detail.clone(),
                // The implicit judgment's rule: this step's `satisfied`
                // verdict (#284 follow-up), mirroring the label used for
                // the CLI reporter's `FailedAssertion`.
                expr: Some(format!("step `{}` satisfied == true", imp.label)),
                // The implicit judgment IS this step's `satisfied`
                // verdict — carry the link so the reporter folds it into
                // the step and propagates the status (#280).
                step_index: Some(imp.step_index as u32),
            })
            .await?;
        if !matches!(imp.state, VerdictState::Pass) {
            failed.push(FailedAssertion {
                expr: format!("implicit: step `{}` satisfied == true", imp.label),
                state: imp.state,
                detail: imp.detail.clone(),
            });
        }
        assertion_outcomes.push(AssertionOutcome {
            assertion_index: index,
            state: imp.state,
            detail: imp.detail,
        });
    }
    Ok(())
}

pub(crate) fn step_label(step: &duhem_schema::Step, idx: usize) -> String {
    step.id
        .clone()
        .unwrap_or_else(|| format!("{} #{idx}", step.uses_name()))
}

/// A step's human-facing label for reports and run views. Authored
/// prose wins, then the reference id, then the action/index fallback.
/// Engine diagnostics deliberately keep using [`step_label`].
pub(crate) fn display_step_label(step: &duhem_schema::Step, idx: usize) -> String {
    step.description
        .clone()
        .unwrap_or_else(|| step_label(step, idx))
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

#[cfg(test)]
mod fail_detail_tests {
    use super::*;
    use serde_json::json;

    fn step(uses: &str) -> duhem_schema::Step {
        serde_yml::from_str(&format!("uses: {uses}\n")).expect("step")
    }

    fn ev(with_yaml: &str, outputs: &[(&str, serde_json::Value)]) -> StepEvidence {
        StepEvidence {
            with: serde_yml::from_str(with_yaml).expect("with"),
            outputs: outputs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            skip_reason: None,
            catalog_reference: None,
        }
    }

    #[test]
    fn assert_element_not_exists_names_locator_and_count() {
        // The reported case (#280): a `not_exists` assertion that fired
        // because the element WAS present. The message must say what it
        // looked for and how many it found — not `actual false`.
        let e = ev(
            r#"{ locator: { text: "Manager" }, expected: not_exists, timeout: "5s" }"#,
            &[("satisfied", json!(false)), ("count", json!(1))],
        );
        assert_eq!(
            assert_element_fail_detail(&e).as_deref(),
            Some("expected text \"Manager\" to be absent within 5s, but 1 still matched"),
        );
    }

    #[test]
    fn assert_element_exists_reports_none_found() {
        let e = ev(
            r##"{ locator: { css: "#email" }, expected: exists }"##,
            &[("satisfied", json!(false)), ("count", json!(0))],
        );
        assert_eq!(
            assert_element_fail_detail(&e).as_deref(),
            Some("expected css #email to appear, but none was found"),
        );
    }

    #[test]
    fn assert_element_visible_distinguishes_present_from_absent() {
        let present = ev(
            r#"{ locator: { role: button, name: Go }, expected: visible, timeout: "2s" }"#,
            &[("satisfied", json!(false)), ("count", json!(3))],
        );
        assert_eq!(
            assert_element_fail_detail(&present).as_deref(),
            Some(
                "expected role=button \"Go\" to be visible within 2s, but it stayed hidden (3 present)"
            ),
        );
        let absent = ev(
            r#"{ locator: { role: button, name: Go }, expected: visible }"#,
            &[("satisfied", json!(false)), ("count", json!(0))],
        );
        assert_eq!(
            assert_element_fail_detail(&absent).as_deref(),
            Some("expected role=button \"Go\" to be visible, but it never appeared"),
        );
    }

    #[test]
    fn catalog_locator_failure_names_entry_and_resolved_locator() {
        let mut e = ev(
            r#"{ locator: { role: button, name: Sign In }, expected: visible }"#,
            &[("satisfied", json!(false)), ("count", json!(0))],
        );
        e.catalog_reference = Some("$pages.login.submit".to_string());
        let detail = assert_element_fail_detail(&e).unwrap();
        assert!(detail.contains("`$pages.login.submit`"), "{detail}");
        assert!(detail.contains("role=button \"Sign In\""), "{detail}");
        assert!(detail.contains("never appeared"), "{detail}");
    }

    #[test]
    fn assert_element_hidden_reports_still_visible() {
        let e = ev(
            r#"{ locator: { testid: banner }, expected: hidden, timeout: "1s" }"#,
            &[("satisfied", json!(false)), ("count", json!(1))],
        );
        assert_eq!(
            assert_element_fail_detail(&e).as_deref(),
            Some(
                "expected testid \"banner\" to be hidden within 1s, but it stayed visible (1 present)"
            ),
        );
    }

    #[test]
    fn poll_intent_names_endpoint_and_last_status() {
        let s = step("api/poll");
        let e = ev(
            r#"{ method: GET, url: "http://x/job", until: "$response.body.done == true", timeout: "30s" }"#,
            &[("satisfied", json!(false)), ("status", json!(500))],
        );
        assert_eq!(
            judging_fail_detail(&s, &e, "api/poll #0"),
            "`api/poll`: expected GET http://x/job until $response.body.done == true within 30s, but it did not hold — last status 500",
        );
    }

    #[test]
    fn unrecognized_action_falls_back_to_generic() {
        // No known `with:` fields → nothing legible → the plain form,
        // still keyed to the step label so it localizes.
        let s = step("custom/thing");
        let e = ev("{}", &[("satisfied", json!(false))]);
        assert_eq!(
            judging_fail_detail(&s, &e, "custom/thing #2"),
            "actual false, expected true (implicit `satisfied` of step `custom/thing #2`)",
        );
    }

    #[test]
    fn assert_element_with_unresolved_locator_falls_back() {
        // A `$`-ref that never resolved leaves a non-locator `with:`;
        // rather than emit garbage, fall back to the generic form.
        let s = step("ui/assert-element");
        let e = ev(
            r#"{ expected: not_exists }"#,
            &[("satisfied", json!(false))],
        );
        assert_eq!(
            judging_fail_detail(&s, &e, "ui/assert-element #7"),
            "actual false, expected true (implicit `satisfied` of step `ui/assert-element #7`)",
        );
    }
}

#[cfg(test)]
mod step_correlation_tests {
    use super::*;

    fn check(yaml: &str) -> duhem_schema::Check {
        serde_yml::from_str(yaml).expect("check")
    }

    #[test]
    fn explicit_assertion_on_one_step_folds_onto_it() {
        // `$steps.update.outputs.status == 200` is about the `update`
        // step (index 1) → folds onto it (#279 follow-up), so the api
        // call that 500'd goes red instead of a green "step ok".
        let c = check(
            "id: AC-1\nsteps:\n  - { id: login, uses: api/call }\n  - { id: update, uses: api/call }\nassertions:\n  - \"$steps.update.outputs.status == 200\"\n",
        );
        let e = assertion_to_expr(&c.assertions[0]);
        assert_eq!(owning_step_index(&e, &c), Some(1));
    }

    #[test]
    fn literal_only_assertion_folds_nowhere() {
        let c =
            check("id: AC-1\nsteps:\n  - { id: a, uses: api/call }\nassertions:\n  - \"1 == 1\"\n");
        let e = assertion_to_expr(&c.assertions[0]);
        assert_eq!(owning_step_index(&e, &c), None);
    }

    #[test]
    fn cross_step_comparison_stays_standalone() {
        // Two distinct steps referenced → no single owner → standalone.
        let c = check(
            "id: AC-1\nsteps:\n  - { id: a, uses: api/call }\n  - { id: b, uses: api/call }\nassertions:\n  - \"$steps.a.outputs.id == $steps.b.outputs.id\"\n",
        );
        let e = assertion_to_expr(&c.assertions[0]);
        assert_eq!(owning_step_index(&e, &c), None);
    }
}

#[cfg(test)]
mod step_label_tests {
    use super::*;

    fn step(yaml: &str) -> duhem_schema::Step {
        serde_yml::from_str(yaml).expect("step")
    }

    #[test]
    fn display_label_prefers_description_then_id_then_action_index() {
        let described = step("id: submit\ndescription: Submit the sign-in form\nuses: ui/click\n");
        assert_eq!(display_step_label(&described, 3), "Submit the sign-in form");
        assert_eq!(
            display_step_label(&step("id: submit\nuses: ui/click\n"), 3),
            "submit"
        );
        assert_eq!(
            display_step_label(&step("uses: ui/click\n"), 3),
            "ui/click #3"
        );
    }

    #[test]
    fn unresolved_reference_diagnostic_stays_id_first() {
        let described = step("id: submit\ndescription: Submit the sign-in form\nuses: ui/click\n");
        let error = EngineError::UnresolvedReference {
            reference: "$inputs.missing".to_string(),
            step: step_label(&described, 0),
            context: String::new(),
        };
        let message = error.to_string();
        assert!(message.contains("step `submit`"), "{message}");
        assert!(!message.contains("Submit the sign-in form"), "{message}");
    }

    #[test]
    fn implicit_failure_uses_the_display_label() {
        let check: duhem_schema::Check = serde_yml::from_str(
            "id: AC-1.1\nsteps:\n  - id: username\n    description: The Username field is offered\n    uses: ui/assert-element\n",
        )
        .expect("check");
        let evidence = vec![StepEvidence {
            with: serde_yml::Value::Null,
            outputs: BTreeMap::from([("satisfied".to_string(), serde_json::json!(false))]),
            skip_reason: None,
            catalog_reference: None,
        }];
        let outcomes = implicit_judgment_outcomes(&check, |_| true, &evidence, false, false, false);
        assert_eq!(outcomes[0].label, "The Username field is offered");
        assert!(
            outcomes[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("The Username field is offered")
        );
    }
}
