//! `Action` — the trait every built-in action type implements.
//!
//! The shape is the structural enforcer of two identity commitments
//! from `CLAUDE.md`:
//!
//! - **Mechanical judgment, not LLM judgment.** `invoke` returns
//!   structured *observations*, not a verdict. The judge interprets
//!   them.
//! - **Holistic Verification Principle.** `ActionCtx` carries a real
//!   Playwright `Page` (built in `browser::CheckBrowser`) for actions
//!   that drive one; there is no in-memory DOM mock injected for
//!   tests. Page-free actions (`api/call`, `db/*`, …) carry `None`.
//!
//! The runtime spec wires `ActionCtx`'s expression evaluator and
//! step-index threading; here we keep the trait shape minimal.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;

use crate::browser::Page;

use crate::error::ActionError;

/// Default `within:` per spec — applies whenever an action's `with:`
/// schema omits an explicit timeout.
pub const DEFAULT_WITHIN: Duration = Duration::from_secs(5);

/// Per-check execution context handed to every `Action::invoke`.
///
/// A new `ActionCtx` is created per check (one Playwright `Page` per
/// check; see `browser::CheckBrowser`). The runtime spec extends
/// this with the expression evaluator and evidence sink — both are
/// out of scope here so we keep the struct narrow.
pub struct ActionCtx<'a> {
    /// Browser page bound to this check, or `None` for a page-free
    /// step. The engine attaches a page only when the check contains a
    /// step that needs one (`uses_requires_page`); `api/call`,
    /// `api/poll`, `api/stream`, `db/*`, and `cli/*` run with `None`.
    pub page: Option<&'a Page>,
    /// Zero-based index of the currently-executing step within its
    /// check. Carried for evidence threading downstream.
    pub step_index: usize,
}

impl<'a> ActionCtx<'a> {
    /// The browser page, for actions whose `requires_page()` is `true`.
    /// The dispatch layer attaches a page before invoking such an action
    /// (and refuses the check otherwise), so the `Err` arm is unreachable
    /// in practice — returning a typed error rather than panicking keeps
    /// a future regression a clean `Inconclusive`, not a crash.
    pub fn require_page(&self) -> Result<&'a Page, ActionError> {
        self.page.ok_or_else(|| {
            ActionError::Playwright(
                "action requires a browser page but none was provisioned".to_string(),
            )
        })
    }
}

/// Per-action lifecycle outcome. Maps onto
/// `AssertionOutcome::{Ok, Error, Inconclusive(Timeout)}` in the
/// judge spec — `Timeout` is the structural reason actions prefer
/// wait-with-timeout over hard-fail polling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Error,
    Timeout,
}

/// One observation captured during an action invocation. Phase 0 is
/// shape-only — concrete kinds (screenshot, DOM snapshot, network
/// trace) land with the evidence spec.
#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub kind: String,
    pub note: Option<String>,
}

/// What an action returns. `outputs` are surfaced as
/// `$steps.<id>.outputs.<name>` to downstream assertions.
#[derive(Debug, Clone, Serialize)]
pub struct ActionResult {
    pub outcome: Outcome,
    pub outputs: BTreeMap<String, serde_json::Value>,
    pub observations: Vec<Observation>,
}

impl ActionResult {
    pub fn ok() -> Self {
        Self {
            outcome: Outcome::Ok,
            outputs: BTreeMap::new(),
            observations: Vec::new(),
        }
    }

    pub fn timeout() -> Self {
        Self {
            outcome: Outcome::Timeout,
            outputs: BTreeMap::new(),
            observations: Vec::new(),
        }
    }

    pub fn error() -> Self {
        Self {
            outcome: Outcome::Error,
            outputs: BTreeMap::new(),
            observations: Vec::new(),
        }
    }

    pub fn with_output(mut self, key: &str, value: serde_json::Value) -> Self {
        self.outputs.insert(key.to_string(), value);
        self
    }
}

/// Whether an action identified by its `Step.uses` string needs a
/// Playwright `Page`. UI actions (`ui/*`) drive a page directly, and
/// `api/observe` reads the page's network recorder. `api/call`,
/// `api/poll`, `api/stream`, `db/*`, and `cli/*` never touch a page.
/// Single source
/// of truth for two consumers: the engine opens a per-check browser
/// only when a step needs it, and the CLI skips launching the
/// Playwright sidecar entirely for page-free verifications.
pub fn uses_requires_page(uses: &str) -> bool {
    uses.starts_with("ui/") || uses == "api/observe"
}

/// Which layer of the delivery web a `Step.uses` action exercises
/// (#192): `ui/*` → `ui`, `api/*` → `api`, `db/*` → `data`,
/// `cli/*` → `runtime`. The tag is derived from the *executed*
/// action's catalog family — never inferred from intent or URLs —
/// and a `uses` outside the catalog families is untagged (`None`)
/// rather than guessed (the mechanical-judgment posture: no
/// inference dressed as evidence). The closed token set is the ④
/// delivery-web-span vocabulary; the runtime stamps it onto
/// `step_started` evidence and the store folds it into `spans`.
pub fn layer_for_uses(uses: &str) -> Option<&'static str> {
    if uses.starts_with("ui/") {
        Some("ui")
    } else if uses.starts_with("api/") {
        Some("api")
    } else if uses.starts_with("db/") {
        Some("data")
    } else if uses.starts_with("cli/") {
        Some("runtime")
    } else {
        None
    }
}

/// One built-in action type. Implementors live under `ui/`, `api/`,
/// `db/`, etc.
///
/// Object-safety via `async-trait`: the runtime dispatches on a
/// `dyn Action` keyed by `Step.uses`, so the future must be boxed.
#[async_trait]
pub trait Action: Send + Sync {
    /// Action-type identifier as it appears in `Step.uses` (e.g.
    /// `"ui/click"`).
    fn uses(&self) -> &'static str;

    /// Whether this action needs a Playwright `Page`. Defaults to the
    /// family classification from [`uses_requires_page`] (`ui/*` →
    /// `true`), so a new action gets the right answer from its `uses`
    /// string alone. The runtime uses it to skip the browser for
    /// page-free checks.
    fn requires_page(&self) -> bool {
        uses_requires_page(self.uses())
    }

    /// Run the action against the per-check context. `with` is the
    /// raw deserialized `Step.with`; the implementation downcasts
    /// to its typed `With` struct via `serde_yml`.
    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError>;
}

#[cfg(test)]
mod tests {
    use super::{ActionCtx, uses_requires_page};

    #[test]
    fn require_page_on_page_free_ctx_is_a_typed_error_not_a_panic() {
        // A page-free context (what the engine hands `api/*`, `db/*`,
        // `cli/*`) yields an `ActionError`, never a panic — the dispatch
        // layer guarantees a page for `requires_page` actions, so this
        // arm is unreachable in practice but stays a clean error.
        let ctx = ActionCtx {
            page: None,
            step_index: 0,
        };
        assert!(ctx.require_page().is_err());
    }

    #[test]
    fn page_need_by_action_family() {
        // UI actions drive a page.
        for u in [
            "ui/navigate",
            "ui/click",
            "ui/type",
            "ui/select",
            "ui/assert-element",
            "ui/assert-url",
            "ui/assert-state",
        ] {
            assert!(uses_requires_page(u), "{u} should need a page");
        }
        // api/observe reads the page's network recorder.
        assert!(uses_requires_page("api/observe"));
        // Everything else is page-free.
        for u in [
            "api/call",
            "api/poll",
            "api/stream",
            "db/query",
            "db/observe",
            "db/seed",
            "cli/invoke",
        ] {
            assert!(!uses_requires_page(u), "{u} should be page-free");
        }
    }
}

#[cfg(test)]
mod layer_tests {
    use super::layer_for_uses;

    #[test]
    fn catalog_families_map_to_their_layers_and_nothing_is_guessed() {
        assert_eq!(layer_for_uses("ui/click"), Some("ui"));
        assert_eq!(layer_for_uses("ui/assert-element"), Some("ui"));
        assert_eq!(layer_for_uses("api/call"), Some("api"));
        assert_eq!(layer_for_uses("api/observe"), Some("api"));
        assert_eq!(layer_for_uses("db/query"), Some("data"));
        assert_eq!(layer_for_uses("db/observe"), Some("data"));
        assert_eq!(layer_for_uses("cli/invoke"), Some("runtime"));
        // Out-of-catalog: untagged, never guessed.
        assert_eq!(layer_for_uses("custom/thing"), None);
        assert_eq!(layer_for_uses(""), None);
    }
}
