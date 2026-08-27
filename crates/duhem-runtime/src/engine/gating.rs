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
