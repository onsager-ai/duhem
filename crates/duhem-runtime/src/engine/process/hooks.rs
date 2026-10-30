//! Interrupted execution drains pending authored cleanup before finalization.
use duhem_schema::{CleanupPolicy, Step};

use super::Owner;
use crate::engine::RunState;

#[derive(Debug, Clone)]
pub(crate) struct CleanupHook {
    pub(crate) owner: Owner,
    pub(crate) steps: Vec<Step>,
    pub(crate) policy: CleanupPolicy,
}

impl RunState {
    pub(crate) fn register_process_cleanup(
        &mut self,
        owner: Owner,
        steps: &[Step],
        policy: CleanupPolicy,
    ) {
        self.process_cleanup.push(CleanupHook {
            owner,
            steps: steps.to_vec(),
            policy,
        });
    }

    pub(crate) fn begin_process_cleanup(&mut self, owner: &Owner) {
        self.process_cleanup.retain(|hook| hook.owner != *owner);
    }
}
