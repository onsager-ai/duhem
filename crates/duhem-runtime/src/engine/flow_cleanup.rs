//! Flow `cleanup:` registration and LIFO unwind (#615).
//!
//! A `call:` to a flow with `cleanup:` is expanded by the loader into plain
//! steps, each carrying the cleanup. The first of them to be dispatched
//! registers it on the run's cleanup-hook stack (`process_cleanup`), the same
//! stack that holds fixture `down:` and block `teardown:` hooks, so one LIFO
//! order covers all of them and an interrupted run drains them alike. The
//! enclosing scope unwinds what was registered since its mark, last first.
//! Each cleanup runs as an ordinary teardown lifecycle block, so it reuses
//! the fixture-`down:` failure and gate semantics.

use std::collections::BTreeMap;

use duhem_actions::RunBrowser;
use duhem_evidence::EvidenceWriter;
use duhem_schema::Step;

use crate::engine::context::RunState;
use crate::engine::process::hooks::{CleanupHook, FlowHook};
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

impl RunState {
    /// Register the cleanups carried by a step that is being dispatched. A
    /// call registers once: later steps of an entered call find it on the
    /// stack. A call inside a `for_each:` body registers once per iteration.
    pub(crate) fn register_flow_cleanups(&self, step: &Step, at: &RegisteredAt) {
        let mut hooks = self.hooks();
        for cleanup in &step.flow_cleanup {
            let entered = hooks
                .iter()
                .filter_map(|hook| hook.flow.as_ref())
                .any(|flow| {
                    flow.cleanup.flow == cleanup.flow
                        && flow.cleanup.call == cleanup.call
                        && (!cleanup.per_iteration || flow.snapshot.is_none())
                });
            if entered {
                continue;
            }
            hooks.push(CleanupHook {
                owner: self.process_owner.clone(),
                steps: Vec::new(),
                policy: cleanup.policy,
                session: self.session.clone(),
                budget: self.context_budget.clone(),
                flow: Some(FlowHook {
                    cleanup: cleanup.clone(),
                    at: at.clone(),
                    snapshot: None,
                }),
            });
        }
    }

    /// Start of a scope: everything pushed from here on belongs to it.
    pub(crate) fn flow_cleanup_mark(&self) -> usize {
        self.hooks().len()
    }

    /// Give every flow registration that has no snapshot yet the values its
    /// call has produced so far. Taken when a block or check body ends, and
    /// (`iteration`) when a loop iteration ends, where it only reaches calls
    /// made inside the loop body, so a later iteration cannot overwrite what
    /// an earlier one read.
    pub(crate) fn snapshot_flow_cleanups(&self, snapshot: &Snapshot, iteration: bool) {
        for flow in self
            .hooks()
            .iter_mut()
            .filter_map(|hook| hook.flow.as_mut())
            .filter(|flow| flow.snapshot.is_none() && (!iteration || flow.cleanup.per_iteration))
        {
            flow.snapshot = Some(snapshot.clone());
        }
    }
}

/// Run one registered flow cleanup against the values its call recorded.
/// With no recorded snapshot (an interrupted block) it sees the run's
/// `$setup` outputs only.
pub(crate) async fn run_flow_hook(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    hook: &FlowHook,
    child_env: &BTreeMap<String, String>,
) -> Result<Vec<CleanupFailure>, EngineError> {
    let mut view = hook.snapshot.clone().unwrap_or_else(|| Snapshot {
        outputs: run.setup_outputs.clone(),
        binding: None,
    });
    for (recorded, alias) in &hook.cleanup.aliases {
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
        &hook.at,
        &hook.cleanup,
        child_env,
    )
    .await;
    run.cleanup_view = None;
    let mut failures = result?;
    for failure in &mut failures {
        failure.step = format!(
            "flow `{}` cleanup (call `{}`): {}",
            hook.cleanup.flow, hook.cleanup.call, failure.step
        );
    }
    Ok(failures)
}

/// Unwind the hooks pushed since `mark`, last first: flow cleanups and
/// fixture `down:` blocks in one order. Every hook is attempted whatever an
/// earlier one did; the first step-local engine error is returned for the
/// caller to report once its own cleanup has run.
pub(crate) async fn drain(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    mark: usize,
    child_env: &BTreeMap<String, String>,
) -> (Vec<CleanupFailure>, Option<EngineError>) {
    let mut pending = {
        let mut hooks = run.hooks();
        if mark >= hooks.len() {
            Vec::new()
        } else {
            hooks.split_off(mark)
        }
    };
    let mut failures = Vec::new();
    let mut error = None;
    while let Some(hook) = pending.pop() {
        if let Some(flow) = &hook.flow {
            match run_flow_hook(writer, registry, browser, run, flow, child_env).await {
                Ok(mut ran) => failures.append(&mut ran),
                Err(step_error) => {
                    error.get_or_insert(step_error);
                }
            }
        } else if let (Some(fixture), Some(check)) = (&hook.owner.fixture, &hook.owner.check) {
            let result = crate::engine::setup::run_fixture_down(
                writer,
                registry,
                browser,
                run,
                fixture,
                check,
                &hook.steps,
                hook.policy,
                child_env,
            )
            .await;
            failures.extend(
                run.processes
                    .lock()
                    .await
                    .finalize(writer, Some(&hook.owner))
                    .await,
            );
            match result {
                Ok(mut ran) => {
                    for failure in &mut ran {
                        failure.step = format!("fixture `{fixture}`: {}", failure.step);
                    }
                    failures.append(&mut ran);
                }
                Err(step_error) => {
                    error.get_or_insert(step_error);
                }
            }
        }
    }
    (failures, error)
}
