//! Action registry — `Step.uses` → action dispatcher.
//!
//! The spec on issue #15 says the v1 registry is keyed by
//! `Step.uses` and contains exactly the three actions from #12
//! (`ui/navigate`, `ui/click`, `ui/assert-element`). Pluggable /
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

use std::collections::BTreeMap;

use async_trait::async_trait;
use duhem_actions::{Action, ActionCtx, ActionError, ActionResult, AssertElement, Click, Navigate};
use playwright::api::Page;

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

    async fn invoke(
        &self,
        page: Option<&Page>,
        step_index: usize,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let page = page.ok_or_else(|| {
            ActionError::Playwright(format!(
                "action `{}` requires a browser page but none was provisioned",
                self.uses
            ))
        })?;
        let ctx = ActionCtx { page, step_index };
        self.action.invoke(&ctx, with).await
    }
}

/// `BTreeMap<&'static str, Box<dyn Dispatch>>` — the registry shape
/// in spec wording, with the dispatch layer made internal.
pub(crate) type ActionRegistry = BTreeMap<&'static str, Box<dyn Dispatch>>;

/// The v1 catalog: the three actions shipped in #12.
pub(crate) fn default_registry() -> ActionRegistry {
    let mut m: ActionRegistry = BTreeMap::new();
    insert(&mut m, ConcreteAction::new(Box::new(Navigate)));
    insert(&mut m, ConcreteAction::new(Box::new(Click)));
    insert(&mut m, ConcreteAction::new(Box::new(AssertElement)));
    m
}

fn insert(m: &mut ActionRegistry, d: ConcreteAction) {
    m.insert(d.uses(), Box::new(d));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_lists_the_three_v1_actions() {
        let m = default_registry();
        let mut keys: Vec<&str> = m.keys().copied().collect();
        keys.sort();
        assert_eq!(keys, vec!["ui/assert-element", "ui/click", "ui/navigate"]);
    }
}
