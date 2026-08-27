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
use std::time::Duration;

use async_trait::async_trait;
use duhem_actions::Page;
use duhem_actions::{
    Action, ActionCtx, ActionError, ActionResult, AssertElement, AssertState, AssertUrl, Call,
    CaptureSession, Click, DbObserve, Invoke, Navigate, Observe, Poll, Query, Seed, Select, Stream,
    Type, Wait,
};
use serde::Deserialize;
use serde::de::Error as _;

#[derive(Deserialize)]
struct WaitWith {
    duration: duhem_schema::DurationSpec,
}

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

    /// Whether this action's contract emits a boolean `satisfied`
    /// output — a *judging* action (spec #253). Judging steps
    /// implicitly assert `satisfied == true` unless the author binds
    /// `satisfied` in the step's `outputs:`. Test stubs default to
    /// `false` and opt in explicitly.
    fn judges(&self) -> bool {
        false
    }

    /// Scalar output paths sensitive by action contract (spec #355).
    /// Test dispatchers default to none and opt in when exercising the
    /// contract-declared path.
    fn secret_outputs(&self) -> Vec<&'static str> {
        Vec::new()
    }

    async fn invoke(
        &self,
        page: Option<&Page>,
        step_index: usize,
        with: &serde_yml::Value,
        child_env: &BTreeMap<String, String>,
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

    fn judges(&self) -> bool {
        self.action.contract().judges()
    }

    fn secret_outputs(&self) -> Vec<&'static str> {
        self.action.contract().secret_outputs
    }

    async fn invoke(
        &self,
        page: Option<&Page>,
        step_index: usize,
        with: &serde_yml::Value,
        child_env: &BTreeMap<String, String>,
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
        // Only `cli/invoke` needs the runtime-owned env layered in, so
        // only it pays for the clone; every other action (the hot
        // UI/API path) invokes against the original `with` by
        // reference.
        if self.uses == "cli/invoke" {
            let mut invocation_with = with.clone();
            inject_child_env(&mut invocation_with, child_env);
            self.action.invoke(&ctx, &invocation_with).await
        } else {
            self.action.invoke(&ctx, with).await
        }
    }
}

pub(crate) fn enforce_wait_ceiling(
    with: &serde_yml::Value,
    max_wait: Duration,
) -> Result<(), ActionError> {
    let duration_label = with
        .as_mapping()
        .and_then(|mapping| mapping.get(serde_yml::Value::String("duration".into())))
        .map(|value| match value {
            serde_yml::Value::String(raw) => raw.clone(),
            _ => serde_yml::to_string(value)
                .unwrap_or_else(|_| "<unprintable>".into())
                .trim()
                .to_owned(),
        });
    let Ok(wait) = serde_yml::from_value::<WaitWith>(with.clone()) else {
        return Ok(()); // The action owns ordinary `with:` validation.
    };
    let duration = Duration::from(wait.duration);
    if duration <= max_wait {
        return Ok(());
    }
    Err(ActionError::InvalidWith {
        action: "ui/wait",
        source: serde_yml::Error::custom(format!(
            "ui/wait duration `{}` exceeds the {} ceiling; raise `defaults.max_wait` in the root manifest, or use `ui/assert-element` with `timeout:` to wait for a condition",
            duration_label.unwrap_or_else(|| format!("{}ms", duration.as_millis())),
            format_duration(max_wait)
        )),
    })
}

fn format_duration(duration: Duration) -> String {
    if duration == Duration::from_secs(60) {
        "60s".to_string()
    } else if duration.subsec_nanos() == 0 && duration.as_secs().is_multiple_of(60) {
        format!("{}m", duration.as_secs() / 60)
    } else if duration.subsec_nanos() == 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

/// Layer runtime-owned process context onto `cli/invoke` after authored
/// fields resolve. Runtime values win so a nested run cannot detach
/// itself from the parent store or lineage.
fn inject_child_env(with: &mut serde_yml::Value, child_env: &BTreeMap<String, String>) {
    if child_env.is_empty() {
        return;
    }
    let Some(with_map) = with.as_mapping_mut() else {
        return;
    };
    let env_key = serde_yml::Value::String("env".to_string());
    let env = with_map
        .entry(env_key)
        .or_insert_with(|| serde_yml::Value::Mapping(Default::default()));
    let Some(env_map) = env.as_mapping_mut() else {
        return;
    };
    for (key, value) in child_env {
        env_map.insert(
            serde_yml::Value::String(key.clone()),
            serde_yml::Value::String(value.clone()),
        );
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
    insert(&mut m, ConcreteAction::new(Box::new(CaptureSession)));
    insert(&mut m, ConcreteAction::new(Box::new(Wait)));
    insert(&mut m, ConcreteAction::new(Box::new(Call)));
    insert(&mut m, ConcreteAction::new(Box::new(Observe)));
    insert(&mut m, ConcreteAction::new(Box::new(Poll)));
    insert(&mut m, ConcreteAction::new(Box::new(Stream)));
    insert(&mut m, ConcreteAction::new(Box::new(Invoke)));
    insert(&mut m, ConcreteAction::new(Box::new(Query)));
    insert(&mut m, ConcreteAction::new(Box::new(DbObserve)));
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
                "db/observe",
                "db/query",
                "db/seed",
                "ui/assert-element",
                "ui/assert-state",
                "ui/assert-url",
                "ui/capture-session",
                "ui/click",
                "ui/navigate",
                "ui/select",
                "ui/type",
                "ui/wait",
            ]
        );
    }

    #[test]
    fn wait_ceiling_is_inclusive_configurable_and_reports_effective_value() {
        let sixty = serde_yml::from_str("duration: 60s").unwrap();
        enforce_wait_ceiling(&sixty, Duration::from_secs(60)).unwrap();

        let sixty_one = serde_yml::from_str("duration: 61s").unwrap();
        let started = std::time::Instant::now();
        let error = enforce_wait_ceiling(&sixty_one, Duration::from_secs(60)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(error.to_string().contains("61s"));
        assert!(error.to_string().contains("60s ceiling"));

        let two_minutes = serde_yml::from_str("duration: 2m").unwrap();
        enforce_wait_ceiling(&two_minutes, Duration::from_secs(300)).unwrap();

        let thirty = serde_yml::from_str("duration: 30s").unwrap();
        let error = enforce_wait_ceiling(&thirty, Duration::from_secs(5)).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("5s ceiling"), "{message}");
        assert!(message.contains("defaults.max_wait"), "{message}");

        let six_minutes = serde_yml::from_str("duration: 6m").unwrap();
        let error = enforce_wait_ceiling(&six_minutes, Duration::from_secs(300)).unwrap_err();
        assert!(error.to_string().contains("5m ceiling"));
    }
}
