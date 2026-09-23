//! Dashboard adapter over the shared lifecycle fold.

use duhem_evidence::{Event, EventPayload, FlowOrigin, StepOutcome, StepPhase};
use duhem_summary::{
    LifecycleEvent, LifecycleFlowOrigin, LifecycleFold, LifecyclePhase, LifecycleScopeSegment,
    LifecycleStepOutcome,
};

use crate::model::LifecycleBlockDetail;

/// The recorded shapes are leaf [], criterion [criterion], check
/// [criterion, check], and fixture [check, fixture]. Older traces may
/// carry a check without its criterion segment.
pub(super) fn encloses_check(
    scope: &[LifecycleScopeSegment],
    criterion_id: &str,
    check_id: &str,
) -> bool {
    let segment = |index: usize, kind: &str, id: &str| {
        scope
            .get(index)
            .is_some_and(|s| s.kind == kind && s.id == id)
    };
    let kind = |index: usize, expected: &str| scope.get(index).is_some_and(|s| s.kind == expected);
    match scope.len() {
        0 => true,
        1 => segment(0, "criterion", criterion_id) || segment(0, "check", check_id),
        2 => {
            (segment(0, "criterion", criterion_id) && segment(1, "check", check_id))
                || (segment(0, "check", check_id) && kind(1, "fixture"))
        }
        3 => {
            segment(0, "criterion", criterion_id)
                && segment(1, "check", check_id)
                && kind(2, "fixture")
        }
        _ => false,
    }
}

pub(super) fn fold(events: &[Event]) -> Vec<LifecycleBlockDetail> {
    let mut fold = LifecycleFold::default();
    let mut timelines: Vec<Vec<Event>> = Vec::new();
    for event in events {
        let Some(normalized) = normalize(event) else {
            continue;
        };
        if let Some(location) = fold.push(normalized) {
            if timelines.len() <= location.block {
                timelines.resize_with(location.block + 1, Vec::new);
            }
            timelines[location.block].push(event.clone());
        }
    }
    fold.into_blocks()
        .into_iter()
        .enumerate()
        .map(|(index, block)| LifecycleBlockDetail {
            block,
            timeline: timelines.get(index).cloned().unwrap_or_default(),
        })
        .collect()
}

fn normalize(event: &Event) -> Option<LifecycleEvent<'_>> {
    let timestamp_ms = event.ts.timestamp_millis();
    Some(match &event.payload {
        EventPayload::SetupStarted {
            phase,
            fixture_name,
            check_id,
            criterion_id,
            ..
        } => LifecycleEvent::Started {
            phase: phase_of(*phase),
            criterion_id: criterion_id.as_deref(),
            check_id: check_id.as_deref(),
            fixture_name: fixture_name.as_deref(),
            started_at: event.ts.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            timestamp_ms,
        },
        EventPayload::SetupStepStarted {
            phase,
            step_index,
            uses,
            fixture_name,
            check_id,
            criterion_id,
            flow,
            ..
        } => LifecycleEvent::StepStarted {
            phase: phase_of(*phase),
            criterion_id: criterion_id.as_deref(),
            check_id: check_id.as_deref(),
            fixture_name: fixture_name.as_deref(),
            index: *step_index,
            uses,
            flow: flow.as_ref().map(flow_of),
            timestamp_ms,
        },
        EventPayload::SetupStepObservation {
            phase,
            fixture_name,
            check_id,
            criterion_id,
            ..
        } => LifecycleEvent::Observation {
            phase: phase_of(*phase),
            criterion_id: criterion_id.as_deref(),
            check_id: check_id.as_deref(),
            fixture_name: fixture_name.as_deref(),
        },
        EventPayload::SetupStepFinished {
            phase,
            step_index,
            outcome,
            detail,
            fixture_name,
            check_id,
            criterion_id,
        } => LifecycleEvent::StepFinished {
            phase: phase_of(*phase),
            criterion_id: criterion_id.as_deref(),
            check_id: check_id.as_deref(),
            fixture_name: fixture_name.as_deref(),
            index: *step_index,
            outcome: outcome_of(outcome),
            detail: detail.clone(),
            timestamp_ms,
        },
        EventPayload::SetupFinished {
            phase,
            aborted,
            fixture_name,
            check_id,
            criterion_id,
        } => LifecycleEvent::Finished {
            phase: phase_of(*phase),
            criterion_id: criterion_id.as_deref(),
            check_id: check_id.as_deref(),
            fixture_name: fixture_name.as_deref(),
            aborted: *aborted,
            timestamp_ms,
        },
        _ => return None,
    })
}

fn phase_of(phase: StepPhase) -> LifecyclePhase {
    match phase {
        StepPhase::Setup => LifecyclePhase::Setup,
        StepPhase::Teardown => LifecyclePhase::Teardown,
    }
}

fn outcome_of(outcome: &StepOutcome) -> LifecycleStepOutcome {
    match outcome {
        StepOutcome::Ok => LifecycleStepOutcome::Ok,
        StepOutcome::Error => LifecycleStepOutcome::Error,
        StepOutcome::Timeout => LifecycleStepOutcome::Timeout,
        StepOutcome::Skipped { reason, .. } => LifecycleStepOutcome::Skipped {
            reason: reason.clone(),
        },
    }
}

fn flow_of(flow: &FlowOrigin) -> LifecycleFlowOrigin {
    LifecycleFlowOrigin {
        name: flow.name.clone(),
        invocation: flow.invocation.clone(),
        inner_index: flow.inner_index,
        iteration: flow.iteration,
    }
}
