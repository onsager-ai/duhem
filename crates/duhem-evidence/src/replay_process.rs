//! Reconstruct blocked bodies from process lifecycle precondition facts.
use crate::{EventPayload, StepOutcome, StepPhase, VerdictState};
use std::collections::BTreeMap;

type Scope = (Option<String>, Option<String>, Option<String>);

#[derive(Default)]
pub(super) struct ProcessSetup {
    actions: BTreeMap<(Scope, u32), bool>,
    failed: BTreeMap<Scope, VerdictState>,
}

impl ProcessSetup {
    pub fn observe(&mut self, event: &EventPayload) {
        match event {
            EventPayload::SetupStarted {
                phase: StepPhase::Setup,
                criterion_id,
                check_id,
                fixture_name,
                ..
            } => {
                self.failed
                    .remove(&(criterion_id.clone(), check_id.clone(), fixture_name.clone()));
            }
            EventPayload::SetupStepStarted {
                phase: StepPhase::Setup,
                criterion_id,
                check_id,
                fixture_name,
                step_index,
                uses,
                ..
            } => {
                let scope = (criterion_id.clone(), check_id.clone(), fixture_name.clone());
                self.actions
                    .insert((scope, *step_index), uses.starts_with("process/"));
            }
            EventPayload::SetupStepFinished {
                phase: StepPhase::Setup,
                criterion_id,
                check_id,
                fixture_name,
                step_index,
                outcome: StepOutcome::Error,
                ..
            } => {
                let scope = (criterion_id.clone(), check_id.clone(), fixture_name.clone());
                if self.actions.get(&(scope.clone(), *step_index)) == Some(&true) {
                    self.failed
                        .entry(scope)
                        .or_insert(VerdictState::Inconclusive(
                            duhem_judge::InconclusiveCause::EnvironmentError,
                        ));
                }
            }
            _ => {}
        }
    }

    pub fn failed_for(&self, criterion: Option<&str>, check: &str) -> Option<VerdictState> {
        self.failed
            .iter()
            .find_map(|((owner_criterion, owner_check, _), state)| {
                (owner_criterion
                    .as_deref()
                    .is_none_or(|owner| Some(owner) == criterion)
                    && owner_check.as_deref().is_none_or(|owner| owner == check))
                .then_some(*state)
            })
    }
}
