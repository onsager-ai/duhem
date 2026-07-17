//! Built-in action library for verification checks.
//!
//! Houses the canonical action types — `ui/*`, `api/*`, `db/*`, etc.
//! — that Verification Definitions reference. UI actions drive a real
//! browser through the official Playwright Node sidecar (see
//! [`browser`]; spec #71), replacing the unmaintained octaltree
//! `playwright` crate. The `Locator` → selector mapping ([`playwright`])
//! stays pure and driver-independent.
//!
//! Authoring lives here; the executor (which sequences action
//! invocations into a check) lives in `duhem-runtime`.

pub mod action;
pub mod api;
pub mod browser;
pub mod cli;
pub mod db;
pub mod error;
pub mod locator;
pub mod playwright;
pub mod ui;
pub mod with;

pub use action::{
    Action, ActionContract, ActionCtx, ActionResult, DEFAULT_WITHIN, FieldSpec, Observation,
    Outcome, layer_for_uses, uses_requires_page,
};
pub use api::{Call, Observe, Poll, Stream};
pub use browser::{
    CheckBrowser, Cookie, ElementState, NetworkBatch, NetworkEvent, Page, PwError, Rect,
    RunBrowser, SelectBy,
};
pub use cli::Invoke;
pub use db::{DbObserve, Query, Seed};
pub use error::ActionError;
pub use locator::{ExistenceState, Locator};
pub use playwright::to_selector;
pub use ui::{AssertElement, AssertState, AssertUrl, Click, Navigate, Select, Type};
pub use with::WithinSpec;

/// Every built-in action's contract — the single source of truth for
/// `duhem describe` / `duhem actions` and validate-time field checking.
pub fn catalog() -> Vec<ActionContract> {
    vec![
        Navigate.contract(),
        Click.contract(),
        Type.contract(),
        Select.contract(),
        AssertElement.contract(),
        AssertUrl.contract(),
        AssertState.contract(),
        Call.contract(),
        Observe.contract(),
        Poll.contract(),
        Stream.contract(),
        Query.contract(),
        DbObserve.contract(),
        Seed.contract(),
        Invoke.contract(),
    ]
}

/// The contract for a single action by its `uses` string, if it exists.
pub fn contract_for(uses: &str) -> Option<ActionContract> {
    catalog().into_iter().find(|c| c.uses == uses)
}
