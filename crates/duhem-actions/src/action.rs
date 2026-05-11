//! `Action` — the trait every built-in action type implements.
//!
//! The shape is the structural enforcer of two identity commitments
//! from `CLAUDE.md`:
//!
//! - **Mechanical judgment, not LLM judgment.** `invoke` returns
//!   structured *observations*, not a verdict. The judge interprets
//!   them.
//! - **Holistic Verification Principle.** `ActionCtx` carries a real
//!   Playwright `Page` (built in `playwright::CheckBrowser`); there
//!   is no in-memory DOM mock injected for tests.
//!
//! The runtime spec wires `ActionCtx`'s expression evaluator and
//! step-index threading; here we keep the trait shape minimal.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use playwright::api::Page;
use serde::Serialize;

use crate::error::ActionError;

/// Default `within:` per spec — applies whenever an action's `with:`
/// schema omits an explicit timeout.
pub const DEFAULT_WITHIN: Duration = Duration::from_secs(5);

/// Per-check execution context handed to every `Action::invoke`.
///
/// A new `ActionCtx` is created per check (one Playwright `Page` per
/// check; see `playwright::CheckBrowser`). The runtime spec extends
/// this with the expression evaluator and evidence sink — both are
/// out of scope here so we keep the struct narrow.
pub struct ActionCtx<'a> {
    /// Browser page bound to this check.
    pub page: &'a Page,
    /// Zero-based index of the currently-executing step within its
    /// check. Carried for evidence threading downstream.
    pub step_index: usize,
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

    /// Run the action against the per-check context. `with` is the
    /// raw deserialized `Step.with`; the implementation downcasts
    /// to its typed `With` struct via `serde_yml`.
    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError>;
}
