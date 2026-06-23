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

pub use action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN, Observation, Outcome};
pub use api::{Call, Observe};
pub use browser::{
    CheckBrowser, Cookie, ElementState, NetworkBatch, NetworkEvent, Page, PwError, RunBrowser,
    SelectBy,
};
pub use cli::Invoke;
pub use db::{Query, Seed};
pub use error::ActionError;
pub use locator::{ExistenceState, Locator};
pub use playwright::to_selector;
pub use ui::{AssertElement, AssertState, AssertUrl, Click, Navigate, Select, Type};
pub use with::WithinSpec;
