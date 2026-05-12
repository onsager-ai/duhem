//! Built-in action library for verification checks.
//!
//! Houses the canonical action types — `ui/*`, `api/*`, `db/*`, etc.
//! — that Verification Definitions reference. UI actions drive a real
//! browser via the `playwright` crate directly from this crate; there
//! is no Node sidecar (resolved on issue #5; detail in
//! `spec(actions): ui/* action types v1`).
//!
//! Authoring lives here; the executor (which sequences action
//! invocations into a check) lives in `duhem-runtime`.

pub mod action;
pub mod api;
pub mod error;
pub mod locator;
pub mod playwright;
pub mod ui;
pub mod with;

pub use action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN, Observation, Outcome};
pub use api::Call;
pub use error::ActionError;
pub use locator::{ExistenceState, Locator};
pub use playwright::{CheckBrowser, RunBrowser, to_selector};
pub use ui::{AssertElement, Click, Navigate};
pub use with::WithinSpec;
