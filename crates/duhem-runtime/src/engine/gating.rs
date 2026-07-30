//! Closed step-gating policy shared by checks and setup.

use std::collections::BTreeMap;

use duhem_actions::Outcome;
use duhem_schema::StepCondition;

pub(crate) fn skip_reason(condition: StepCondition, failed_by: Option<&str>) -> Option<String> {
    match (condition, failed_by) {
        (StepCondition::Success, Some(step)) => Some(format!("blocked by failed step `{step}`")),
        (StepCondition::Failure, None) => {
            Some("`if: failure` requires an earlier failed step".to_string())
        }
        (StepCondition::Success | StepCondition::Always, None)
        | (StepCondition::Always | StepCondition::Failure, Some(_)) => None,
    }
}

pub(crate) fn step_failed(
    outcome: &Outcome,
    judges: bool,
    outputs: &BTreeMap<String, serde_json::Value>,
) -> bool {
    matches!(outcome, Outcome::Error | Outcome::Timeout)
        || (judges
            && outputs
                .get("satisfied")
                .and_then(serde_json::Value::as_bool)
                == Some(false))
}
