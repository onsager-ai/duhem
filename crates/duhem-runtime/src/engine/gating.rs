//! Closed step-gating policy shared by checks and setup.

use duhem_actions::Outcome;
use duhem_judge::VerdictState;
use duhem_schema::StepCondition;

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
/// Check-step expression authoring remains blocked by schema validation
/// until Tier 2; both runners use this evidence path so that boundary can
/// open without making conditional absence invisible.
pub(super) fn evaluate(
    step: &duhem_schema::Step,
    index: usize,
    failed_by: Option<&str>,
    ctx: &dyn crate::EvalContext,
) -> Result<Option<duhem_evidence::StepOutcome>, super::EngineError> {
    use duhem_evidence::StepOutcome;
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
        crate::EvalResult::Inconclusive(cause) => Err(super::EngineError::UnresolvedReference {
            reference: expr.raw.clone(),
            context: format!(" (condition could not be evaluated: {cause:?})"),
            step: super::outcome::step_label(step, index),
        }),
    }
}
