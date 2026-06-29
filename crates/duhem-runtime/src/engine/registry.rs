//! Action registry — `Step.uses` → action dispatcher.
//!
//! The v1 registry is keyed by `Step.uses` and contains the closed
//! action catalog: the full `ui/*` slice (three from #12, four from
//! #37) plus `api/call` from the spec on issue #21. Pluggable /
//! user-defined `uses:` is §10.8 and lands in Phase 2+. The registry
//! is `pub(crate)`-shaped: external callers only see
//! [`Engine::new`](super::Engine), which wires the default catalog.
//!
//! Internally we dispatch through a thin [`Dispatch`] trait rather
//! than holding `Box<dyn Action>` directly. Same registry semantics,
//! one extra layer of indirection — which lets `#[cfg(test)]` stubs
//! invoke without needing a real Playwright `Page` (test-only stubs
//! live under `#[cfg(test)]`, per spec). The production wrapper
//! borrows the per-check `Page` and dispatches to the real `Action`.
//!
//! `api/call` is registered the same way `ui/*` actions are — through
//! the default [`Dispatch::requires_page`] of `true`, even though the
//! action itself ignores `ActionCtx.page`. Per spec on issue #21 the
//! per-check `CheckBrowser` is still opened for API-only checks;
//! stripping the browser when no `ui/*` step is present is an
//! optimization deferred to a follow-up spec.

use std::collections::BTreeMap;

use async_trait::async_trait;
use duhem_actions::Page;
use duhem_actions::{
    Action, ActionCtx, ActionError, ActionResult, AssertElement, AssertState, AssertUrl, Call,
    Click, Invoke, Navigate, Observe, Poll, Query, Seed, Select, Stream, Type,
};

/// Engine-internal dispatcher. One implementor per registered action
/// (`Step.uses`).
#[async_trait]
pub(crate) trait Dispatch: Send + Sync {
    fn uses(&self) -> &'static str;

    /// Whether invocation requires a Playwright `Page`. Production
    /// wrappers around `duhem-actions::Action` default to `true`
    /// (every v1 action is UI-backed); test stubs override to `false`
    /// when they don't actually drive a browser. Lets the engine
    /// distinguish "we tried to run a UI step without a browser"
    /// (an environment failure) from "the test stub just ran".
    fn requires_page(&self) -> bool {
        true
    }

    async fn invoke(
        &self,
        page: Option<&Page>,
        step_index: usize,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError>;
}

/// Production wrapper around a `Box<dyn Action>` from `duhem-actions`.
pub(crate) struct ConcreteAction {
    uses: &'static str,
    action: Box<dyn Action>,
}

impl ConcreteAction {
    pub fn new(action: Box<dyn Action>) -> Self {
        let uses = action.uses();
        Self { uses, action }
    }
}

#[async_trait]
impl Dispatch for ConcreteAction {
    fn uses(&self) -> &'static str {
        self.uses
    }

    /// Delegate to the wrapped action's real page-need so the engine
    /// opens a per-check browser only when a step actually drives a
    /// page (`ui/*`, `api/observe`) rather than for every action.
    fn requires_page(&self) -> bool {
        self.action.requires_page()
    }

    async fn invoke(
        &self,
        page: Option<&Page>,
        step_index: usize,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        // A page-requiring action without a page is a dispatch-layer
        // failure (the engine should have refused the check upstream).
        if self.action.requires_page() && page.is_none() {
            return Err(ActionError::Playwright(format!(
                "action `{}` requires a browser page but none was provisioned",
                self.uses
            )));
        }
        let ctx = ActionCtx { page, step_index };
        self.action.invoke(&ctx, with).await
    }
}

/// `BTreeMap<&'static str, Box<dyn Dispatch>>` — the registry shape
/// in spec wording, with the dispatch layer made internal.
pub(crate) type ActionRegistry = BTreeMap<&'static str, Box<dyn Dispatch>>;

/// The v1 catalog: the full `ui/*` slice (`ui/navigate`, `ui/click`,
/// `ui/assert-element`, `ui/type`, `ui/select`, `ui/assert-url`,
/// `ui/assert-state`) and `api/call`.
pub(crate) fn default_registry() -> ActionRegistry {
    let mut m: ActionRegistry = BTreeMap::new();
    insert(&mut m, ConcreteAction::new(Box::new(Navigate)));
    insert(&mut m, ConcreteAction::new(Box::new(Click)));
    insert(&mut m, ConcreteAction::new(Box::new(AssertElement)));
    insert(&mut m, ConcreteAction::new(Box::new(Type)));
    insert(&mut m, ConcreteAction::new(Box::new(Select)));
    insert(&mut m, ConcreteAction::new(Box::new(AssertUrl)));
    insert(&mut m, ConcreteAction::new(Box::new(AssertState)));
    insert(&mut m, ConcreteAction::new(Box::new(Call)));
    insert(&mut m, ConcreteAction::new(Box::new(Observe)));
    insert(&mut m, ConcreteAction::new(Box::new(Poll)));
    insert(&mut m, ConcreteAction::new(Box::new(Stream)));
    insert(&mut m, ConcreteAction::new(Box::new(Invoke)));
    insert(&mut m, ConcreteAction::new(Box::new(Query)));
    insert(&mut m, ConcreteAction::new(Box::new(Seed)));
    m
}

fn insert(m: &mut ActionRegistry, d: ConcreteAction) {
    m.insert(d.uses(), Box::new(d));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_lists_the_v1_actions() {
        let m = default_registry();
        let mut keys: Vec<&str> = m.keys().copied().collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "api/call",
                "api/observe",
                "api/poll",
                "api/stream",
                "cli/invoke",
                "db/query",
                "db/seed",
                "ui/assert-element",
                "ui/assert-state",
                "ui/assert-url",
                "ui/click",
                "ui/navigate",
                "ui/select",
                "ui/type",
            ]
        );
    }
}
