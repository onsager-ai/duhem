//! Flow `cleanup:` registration and LIFO unwind (#615).
//!
//! A `call:` to a flow with `cleanup:` is expanded by the loader into plain
//! steps, with the cleanup carried on the call's first expanded step. The
//! runtime registers it here when that step is dispatched and drains the
//! enclosing scope's registrations, last first, at the scope's exit. Each
//! cleanup runs as an ordinary teardown lifecycle block, so it reuses the
//! fixture-`down:` failure and gate semantics.

use std::collections::BTreeMap;

use duhem_actions::RunBrowser;
use duhem_evidence::EvidenceWriter;
use duhem_schema::{FlowCleanup, Step};

use crate::engine::context::RunState;
use crate::engine::registry::ActionRegistry;
use crate::engine::runner::{CleanupFailure, EngineError};
use crate::eval::Value;

/// The lifecycle scope a cleanup was registered at.
#[derive(Debug, Clone)]
pub(crate) enum RegisteredAt {
    Leaf,
    Criterion(String),
    Check(String, String),
}

/// What a cleanup reads from its call: the body's `$steps.*` outputs and, for
/// a `for_each:` iteration, the loop binding.
#[derive(Debug, Clone, Default)]
pub(crate) struct Snapshot {
    pub(crate) outputs: BTreeMap<(String, String), Value>,
    pub(crate) binding: Option<(String, Value)>,
}

#[derive(Debug, Clone)]
pub(crate) struct Registered {
    cleanup: FlowCleanup,
    at: RegisteredAt,
    snapshot: Option<Snapshot>,
}

impl RunState {
    /// Register the cleanups carried by a step that is being dispatched.
    pub(crate) fn register_flow_cleanups(&self, step: &Step, at: &RegisteredAt) {
        if step.flow_cleanup.is_empty() {
            return;
        }
        self.flow_cleanups
            .lock()
            .expect("flow cleanup stack")
            .extend(step.flow_cleanup.iter().map(|cleanup| Registered {
                cleanup: cleanup.clone(),
                at: at.clone(),
                snapshot: None,
            }));
    }

    /// Start of a scope: everything registered from here on belongs to it.
    pub(crate) fn flow_cleanup_mark(&self) -> usize {
        self.flow_cleanups.lock().expect("flow cleanup stack").len()
    }

    /// Give every registration that has no snapshot yet the values its call
    /// has produced so far. Taken when a block, loop iteration or check body
    /// ends, so a later iteration cannot overwrite what an earlier one read.
    pub(crate) fn snapshot_flow_cleanups(&self, snapshot: &Snapshot) {
        for registered in self
            .flow_cleanups
            .lock()
            .expect("flow cleanup stack")
            .iter_mut()
            .filter(|registered| registered.snapshot.is_none())
        {
            registered.snapshot = Some(snapshot.clone());
        }
    }

    pub(crate) fn take_flow_cleanups(&self, mark: usize) -> Vec<Registered> {
        let mut stack = self.flow_cleanups.lock().expect("flow cleanup stack");
        if mark >= stack.len() {
            return Vec::new();
        }
        stack.split_off(mark)
    }
}

/// Unwind the cleanups registered since `mark`, last first. Every cleanup is
/// attempted whatever an earlier one did; the first step-local engine error
/// is returned for the caller to report once its own cleanup has run.
pub(crate) async fn drain(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    mark: usize,
    child_env: &BTreeMap<String, String>,
) -> (Vec<CleanupFailure>, Option<EngineError>) {
    let mut registered = run.take_flow_cleanups(mark);
    let mut failures = Vec::new();
    let mut error = None;
    while let Some(next) = registered.pop() {
        let mut view = next.snapshot.unwrap_or_default();
        for (recorded, alias) in &next.cleanup.aliases {
            let renamed: Vec<_> = view
                .outputs
                .iter()
                .filter(|((id, _), _)| id == recorded)
                .map(|((_, name), value)| ((alias.clone(), name.clone()), value.clone()))
                .collect();
            view.outputs.extend(renamed);
        }
        run.cleanup_view = Some(view);
        let result = crate::engine::setup::run_flow_cleanup(
            writer,
            registry,
            browser,
            run,
            &next.at,
            &next.cleanup,
            child_env,
        )
        .await;
        run.cleanup_view = None;
        match result {
            Ok(mut ran) => {
                for failure in &mut ran {
                    failure.step = format!(
                        "flow `{}` cleanup (call `{}`): {}",
                        next.cleanup.flow, next.cleanup.call, failure.step
                    );
                }
                failures.append(&mut ran);
            }
            Err(step_error) => {
                error.get_or_insert(step_error);
            }
        }
    }
    (failures, error)
}
