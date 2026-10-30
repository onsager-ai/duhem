use duhem_evidence::{EventPayload, EvidenceWriter, StepOutcome, StepPhase};

use super::{Entry, Owner, Processes, error};
use crate::engine::CleanupFailure;

impl Owner {
    fn evidence_owner(&self) -> duhem_evidence::event::ProcessOwner {
        duhem_evidence::event::ProcessOwner {
            scope: if self.fixture.is_some() {
                "fixture"
            } else if self.check.is_some() {
                "check"
            } else if self.criterion.is_some() {
                "criterion"
            } else {
                "leaf"
            }
            .into(),
            run_scope: self.run_scope.clone(),
            criterion_id: self.criterion.clone(),
            check_id: self.check.clone(),
            attempt: self.attempt.clone(),
            fixture_name: self.fixture.clone(),
        }
    }
}

impl Entry {
    pub(super) async fn record(
        &self,
        writer: &mut EvidenceWriter,
        operation: &str,
        failure: Option<&duhem_actions::ActionError>,
    ) -> Result<(), duhem_actions::ActionError> {
        writer
            .append(EventPayload::ProcessObserved {
                owner: self.owner.evidence_owner(),
                handle: self.handle.clone(),
                generation: self.generation,
                pid: self.pid,
                operation: operation.into(),
                facts: self.snapshot(0),
                detail: failure.map(|failure| writer.mask_text(&failure.to_string())),
            })
            .await
            .map_err(|err| error(format!("record process evidence: {err}")))?;
        Ok(())
    }
}

impl Processes {
    /// Always required, regardless of an authored fixture cleanup policy or
    /// --keep-env. A None owner drains all remaining scopes after cancellation
    /// or an engine/evidence error. Stop continues even if evidence fails.
    pub async fn finalize(
        &mut self,
        writer: &mut EvidenceWriter,
        owner: Option<&Owner>,
    ) -> Vec<CleanupFailure> {
        let mut failures = Vec::new();
        for entry in self.entries.iter_mut().rev().filter(|entry| {
            owner.is_none_or(|owner| {
                entry.owner == *owner
                    || (owner.fixture.is_none()
                        && entry.owner.may_read(owner)
                        && (owner.attempt.is_some() || owner.check.is_none()))
            })
        }) {
            if entry.finalized || (entry.stopped && entry.stop_result.is_some()) {
                continue;
            }
            let scope = entry.owner.clone();
            let mut evidence_error = None;
            macro_rules! append {
                ($payload:expr) => {
                    if let Err(err) = writer.append($payload).await {
                        evidence_error.get_or_insert_with(|| writer.mask_text(&err.to_string()));
                    }
                };
            }
            append!(EventPayload::SetupStarted {
                cleanup_policy: Some("required".into()),
                phase: StepPhase::Teardown,
                step_count: 1,
                fixture_name: scope.fixture.clone(),
                check_id: scope.check.clone(),
                criterion_id: scope.criterion.clone(),
            });
            append!(EventPayload::SetupStepStarted {
                phase: StepPhase::Teardown,
                step_index: 0,
                uses: "process/stop".into(),
                layer: Some("runtime".into()),
                with: std::collections::BTreeMap::from([
                    ("handle".into(), entry.handle.clone().into()),
                    ("finalizer".into(), true.into())
                ]),
                fixture_name: scope.fixture.clone(),
                check_id: scope.check.clone(),
                criterion_id: scope.criterion.clone(),
                flow: None,
            });
            let result = entry.stop().await;
            if let Err(err) = entry
                .record(writer, "finalizer", result.as_ref().err())
                .await
            {
                evidence_error.get_or_insert_with(|| writer.mask_text(&err.to_string()));
            }
            let detail = result
                .as_ref()
                .err()
                .map(|err| writer.mask_text(&err.to_string()))
                .or_else(|| evidence_error.clone());
            let outcome = if detail.is_some() {
                StepOutcome::Error
            } else {
                StepOutcome::Ok
            };
            append!(EventPayload::SetupStepFinished {
                judgment: None,
                phase: StepPhase::Teardown,
                step_index: 0,
                outcome: outcome.clone(),
                detail: detail.clone(),
                fixture_name: scope.fixture.clone(),
                check_id: scope.check.clone(),
                criterion_id: scope.criterion.clone(),
            });
            append!(EventPayload::SetupFinished {
                phase: StepPhase::Teardown,
                aborted: false,
                fixture_name: scope.fixture.clone(),
                check_id: scope.check.clone(),
                criterion_id: scope.criterion.clone(),
            });
            entry.finalized = true;
            if detail.is_some() || evidence_error.is_some() {
                failures.push(CleanupFailure {
                    required: true,
                    step: format!(
                        "owned process {} generation {} finalizer",
                        entry.handle, entry.generation
                    ),
                    outcome: StepOutcome::Error,
                    detail: detail.or(evidence_error),
                });
            }
        }
        failures
    }
}
