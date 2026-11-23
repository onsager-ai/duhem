//! Project non-verdict scores from masked, recorded check-step evidence.

use duhem_summary::{Score, Scores};
use serde_json::Number;

use crate::{Event, EventPayload, ObservationValue, StepOutcome};

struct ScoreStep<'a> {
    criterion: &'a str,
    check: &'a str,
    index: u32,
    id: Option<&'a str>,
    value: Option<&'a Number>,
}

/// Keep the last completed execution of each score step, including skipped
/// executions. Earlier loop iterations and retry attempts remain in the trace.
pub fn scores(events: &[Event]) -> Scores {
    let mut scores = Scores::new();
    let mut step = None;
    for event in events {
        match &event.payload {
            EventPayload::StepStarted {
                criterion_id,
                check_id,
                step_index,
                uses,
                ..
            } => {
                step = (uses == "llm/score").then_some(ScoreStep {
                    criterion: criterion_id,
                    check: check_id,
                    index: *step_index,
                    id: None,
                    value: None,
                });
            }
            EventPayload::StepObservation {
                step_index,
                output_name,
                value: ObservationValue::Inline { value },
            } => {
                if let Some(step) = step.as_mut()
                    && *step_index == step.index
                {
                    match (output_name.as_str(), value) {
                        ("score_step_id", serde_json::Value::String(id)) => step.id = Some(id),
                        ("score", serde_json::Value::Number(value)) => step.value = Some(value),
                        _ => {}
                    }
                }
            }
            EventPayload::StepFinished {
                step_index,
                outcome,
                detail,
            } if step.as_ref().is_some_and(|step| step.index == *step_index) => {
                let step = step.take().expect("matched active score step");
                let Some(id) = step.id else { continue };
                let score = match (outcome, step.value) {
                    (StepOutcome::Ok, Some(value)) => Score::Present {
                        value: value.clone(),
                    },
                    _ => {
                        let cause = match outcome {
                            StepOutcome::Skipped { reason, .. } => reason,
                            StepOutcome::Error => detail.as_deref().unwrap_or("score step failed"),
                            StepOutcome::Timeout => {
                                detail.as_deref().unwrap_or("score step timed out")
                            }
                            StepOutcome::Ok => "score step recorded no numeric value",
                        };
                        Score::Missing {
                            missing_cause: cause.to_owned(),
                        }
                    }
                };
                scores
                    .entry(step.criterion.to_owned())
                    .or_default()
                    .entry(step.check.to_owned())
                    .or_default()
                    .insert(id.to_owned(), score);
            }
            _ => {}
        }
    }
    scores
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn score_ids_are_scoped_to_checks_and_latest_execution_wins() {
        let mut events = Vec::new();
        let mut record = |mut event: Value| {
            event["ts"] = json!("2026-11-23T00:00:00.000Z");
            event["seq"] = json!(events.len());
            events.push(serde_json::from_value(event).unwrap());
        };
        for (criterion, check, value) in [
            ("A", "common", 4),
            ("A", "other", 1),
            ("B", "common", 2),
            ("A", "common", 3),
        ] {
            record(json!({
                "kind": "step_started", "uses": "llm/score", "step_index": 0,
                "criterion_id": criterion, "check_id": check
            }));
            record(json!({
                "kind": "step_observation", "step_index": 0,
                "output_name": "score_step_id", "value": "quality"
            }));
            record(json!({
                "kind": "step_observation", "step_index": 0,
                "output_name": "score", "value": value
            }));
            record(json!({"kind": "step_finished", "step_index": 0, "outcome": "ok"}));
        }
        assert!(scores(&[]).is_empty());
        assert_eq!(
            serde_json::to_value(scores(&events)).unwrap(),
            json!({
                "A": {"common": {"quality": {"value": 3}}, "other": {"quality": {"value": 1}}},
                "B": {"common": {"quality": {"value": 2}}}
            })
        );
    }
}
