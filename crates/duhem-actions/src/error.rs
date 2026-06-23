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

    /// HTTP transport-layer failure (DNS, TCP, TLS, malformed method
    /// or URL). Timeouts do *not* land here — they surface as a
    /// successful return with `Outcome::Timeout`. This variant is the
    /// `api/*` analogue of `Playwright`: structural failure that the
    /// engine maps to `Outcome::Error`.
    #[error("http transport error: {0}")]
    Http(String),

    /// `cli/invoke` could not spawn the command or failed mid-stream
    /// (binary not on `PATH`, permission denied, broken pipe writing
    /// stdin, I/O error reading output). A non-zero *exit code* is
    /// **not** an error — it surfaces as `Outcome::Ok` with the code in
    /// the `exit_code` output, the same way a `500` is data for
    /// `api/call`. Only the process never producing an exit code lands
    /// here. The engine maps it to `Outcome::Error`.
    #[error("cli process error: {0}")]
    Process(String),

    /// `db/*` failure: bad connection URL, connect failure, or a SQL
    /// error. As with `api/call`'s status, the *result rows* are data —
    /// only a failure to run the query lands here. The engine maps it to
    /// `Outcome::Error`.
    #[error("db error: {0}")]
    Db(String),
}
