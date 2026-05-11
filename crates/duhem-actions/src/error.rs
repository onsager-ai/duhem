//! `ActionError` — surfaced when an action's `with:` is malformed
//! or its underlying driver fails for a reason that isn't a timeout.
//!
//! Timeouts are *not* errors — they're a normal `Outcome::Timeout`
//! return so the judge can map them to `Inconclusive(Timeout)`. Only
//! structural problems (bad schema, driver crash, browser binary
//! missing) propagate here.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ActionError {
    /// `Step.with` could not be deserialized into the action's
    /// typed `With` struct.
    #[error("invalid `with:` for action `{action}`: {source}")]
    InvalidWith {
        action: &'static str,
        #[source]
        source: serde_yml::Error,
    },

    /// Playwright driver returned a non-timeout error (e.g. browser
    /// binary missing — `playwright::RunBrowser::launch` rewrites
    /// that one with the install hint via `humanize_launch_error`).
    #[error("playwright driver error: {0}")]
    Playwright(String),
}
