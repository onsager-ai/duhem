//! Closed step-gating policy shared by checks and setup.

use duhem_actions::Outcome;
use duhem_judge::VerdictState;
use duhem_schema::{StepCondition, StepGroupFrame};
use std::collections::BTreeMap;

/// Entry-time decisions of the step groups (#614) in one step sequence.
/// A group's `if:` is evaluated once, when its first step is reached; every
/// later step in the group reuses that decision, while its own `if:` still
/// reads the shared failure state.
#[derive(Clone, Default)]
pub(crate) struct GroupGates(BTreeMap<u32, GroupEntry>);

#[derive(Clone)]
enum GroupEntry {
    Open,
    Skipped(duhem_evidence::StepOutcome),
    Unresolved(crate::InconclusiveCause),
}

fn enter(
    frame: &StepGroupFrame,
    failed_by: Option<&str>,
    ctx: &dyn crate::EvalContext,
) -> GroupEntry {
    use duhem_evidence::StepOutcome;
    let named = |reason: String| format!("group `{}`: {reason}", frame.label);
    if let Some(reason) = skip_reason(&frame.condition, failed_by) {
        return GroupEntry::Skipped(StepOutcome::Skipped {
            reason: named(reason),
            condition: None,
            operands: None,
        });
    }
    let StepCondition::Expr(expr) = &frame.condition else {
        return GroupEntry::Open;
    };
    let (result, operands) = crate::eval::operands::evaluate(expr, ctx);
    match result {
        crate::EvalResult::True => GroupEntry::Open,
        crate::EvalResult::False => GroupEntry::Skipped(StepOutcome::Skipped {
            reason: named(format!("condition `{}` evaluated false", expr.raw)),
            condition: Some(expr.raw.clone()),
            operands: Some(operands),
        }),
        crate::EvalResult::Inconclusive(cause) => GroupEntry::Unresolved(cause),
    }
}

pub(crate) fn skip_reason(condition: &StepCondition, failed_by: Option<&str>) -> Option<String> {
    match (condition, failed_by) {
        // A value expression carries `success` semantics for the outcome
        // gate: omitting `if:` means `success`, so replacing it with an
        // expression must not silently forfeit that protection. The
        // expression is evaluated only once this gate passes.
        (StepCondition::Success | StepCondition::Expr(_), Some(step)) => {
            Some(format!("blocked by failed step `{step}`"))
        }
        (StepCondition::Failure, None) => {
            Some("`if: failure` requires an earlier failed step".to_string())
        }
        (StepCondition::Success | StepCondition::Always | StepCondition::Expr(_), None)
        | (StepCondition::Always | StepCondition::Failure, Some(_)) => None,
    }
}

pub(crate) fn step_failed(outcome: &Outcome, judgment: Option<VerdictState>) -> bool {
    matches!(outcome, Outcome::Error | Outcome::Timeout)
        || matches!(judgment, Some(VerdictState::Fail))
}

/// Apply the outcome gate first, then observe the value condition once.
/// Lifecycle and `for_each` steps treat an unresolvable condition as an
/// engine error (the step fails and its dependents go inconclusive).
pub(super) fn evaluate(
    step: &duhem_schema::Step,
    index: usize,
    failed_by: Option<&str>,
    groups: &mut GroupGates,
    ctx: &dyn crate::EvalContext,
) -> Result<Option<duhem_evidence::StepOutcome>, super::EngineError> {
    gate(step, failed_by, groups, ctx).map_err(|cause| super::EngineError::UnresolvedReference {
        reference: condition_source(step),
        context: format!(" (condition could not be evaluated: {cause:?})"),
        step: super::outcome::step_label(step, index),
    })
}

/// Check-step counterpart of [`evaluate`] (#440 R2): an unresolvable
/// condition is not a run abort. The caller records the returned detail
/// as an errored step, which the check's implicit judgment turns into
/// `Inconclusive(MissingObservation)`.
pub(super) fn evaluate_in_check(
    step: &duhem_schema::Step,
    index: usize,
    failed_by: Option<&str>,
    groups: &mut GroupGates,
    ctx: &dyn crate::EvalContext,
) -> Result<Option<duhem_evidence::StepOutcome>, String> {
    gate(step, failed_by, groups, ctx).map_err(|cause| {
        format!(
            "step `{}`: condition `{}` could not be resolved: {}",
            super::outcome::step_label(step, index),
            condition_source(step),
            super::translate::eval_cause_detail(&cause)
        )
    })
}

fn condition_source(step: &duhem_schema::Step) -> String {
    match &step.condition {
        StepCondition::Expr(expr) => expr.raw.clone(),
        _ => String::new(),
    }
}

fn gate(
    step: &duhem_schema::Step,
    failed_by: Option<&str>,
    groups: &mut GroupGates,
    ctx: &dyn crate::EvalContext,
) -> Result<Option<duhem_evidence::StepOutcome>, crate::InconclusiveCause> {
    use duhem_evidence::StepOutcome;
    // Outermost group first: an inner group is entered only once every
    // enclosing group has let its steps run.
    for frame in &step.group {
        let entry = groups
            .0
            .entry(frame.key)
            .or_insert_with(|| enter(frame, failed_by, ctx))
            .clone();
        match entry {
            GroupEntry::Open => {}
            GroupEntry::Skipped(outcome) => return Ok(Some(outcome)),
            GroupEntry::Unresolved(cause) => return Err(cause),
        }
    }
    if let Some(reason) = skip_reason(&step.condition, failed_by) {
        return Ok(Some(StepOutcome::Skipped {
            reason,
            condition: None,
            operands: None,
        }));
    }
    let StepCondition::Expr(expr) = &step.condition else {
        return Ok(None);
    };
    let (result, operands) = crate::eval::operands::evaluate(expr, ctx);
    match result {
        crate::EvalResult::True => Ok(None),
        crate::EvalResult::False => Ok(Some(StepOutcome::Skipped {
            reason: format!("condition `{}` evaluated false", expr.raw),
            condition: Some(expr.raw.clone()),
            operands: Some(operands),
        })),
        crate::EvalResult::Inconclusive(cause) => Err(cause),
    }
}
