//! Dashboard adapter over the shared lifecycle fold.

use duhem_evidence::{Event, EventPayload, FlowOrigin, StepOutcome, StepPhase};
use duhem_summary::{
    LifecycleEvent, LifecycleFlowOrigin, LifecycleFold, LifecyclePhase, LifecycleStepOutcome,
};

use crate::model::LifecycleBlockDetail;

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
