//! Pending authored cleanup, unwound LIFO on the normal path and drained
//! before finalization when a run is interrupted.
use duhem_schema::{FlowCleanup, Step};

use super::Owner;
use crate::engine::RunState;
use crate::engine::flow_cleanup::{RegisteredAt, Snapshot};

/// A flow `cleanup:` waiting on the stack (#615), with the values it reads
/// once its call's scope has recorded them.
#[derive(Debug, Clone)]
pub(crate) struct FlowHook {
    pub(crate) cleanup: FlowCleanup,
    pub(crate) at: RegisteredAt,
    pub(crate) snapshot: Option<Snapshot>,
}

#[derive(Debug, Clone)]
pub(crate) struct CleanupHook {
    pub(crate) owner: Owner,
    pub(crate) steps: Vec<Step>,
    pub(crate) session: Option<std::sync::Arc<crate::engine::session_scope::SessionScope>>,
    pub(crate) budget: Option<std::sync::Arc<crate::engine::session_scope::ContextBudget>>,
    /// Set for a flow `cleanup:`; `steps` is then unused.
    pub(crate) flow: Option<FlowHook>,
}

impl RunState {
    pub(crate) fn register_process_cleanup(&mut self, owner: Owner, steps: &[Step]) {
        let hook = CleanupHook {
            owner,
            steps: steps.to_vec(),
            session: self.session.clone(),
            budget: self.context_budget.clone(),
            flow: None,
        };
        self.hooks().push(hook);
    }

    /// Owner-scoped removal of a block's own hook. Flow cleanups are only
    /// ever removed by being drained.
    pub(crate) fn begin_process_cleanup(&mut self, owner: &Owner) {
        self.hooks()
            .retain(|hook| hook.flow.is_some() || hook.owner != *owner);
    }

    pub(crate) fn hooks(&self) -> std::sync::MutexGuard<'_, Vec<CleanupHook>> {
        self.process_cleanup.lock().expect("cleanup hook stack")
    }
}
