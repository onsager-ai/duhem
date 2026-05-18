//! `ui/*` actions — the Playwright-backed half of the v1 catalog.
//!
//! Each action is a unit struct implementing [`crate::Action`] and a
//! private `With` struct that types `Step.with` at the per-action
//! schema. The trait + lifecycle are deliberately the only thing the
//! runtime knows about; the whole `ui/*` catalog from
//! `docs/duhem-spec.md` §10.5 is just more files in this module —
//! no trait change.
//!
//! ## §10.5 catalog status
//!
//! Shipped in #12 (minimal slice):
//! - `ui/navigate`
//! - `ui/click`
//! - `ui/assert-element`
//!
//! Shipped in #37 (rest of slice):
//! - `ui/type`
//! - `ui/select`
//! - `ui/assert-url`
//! - `ui/assert-state`
//!
//! ## Waiter-action outcome shape
//!
//! The three "waiter" actions diverge on the verdict shape they
//! emit when their deadline elapses without the expectation being
//! met. The divergence is intentional but easy to miss:
//!
//! - `ui/assert-element` and `ui/assert-state`: `Outcome::Ok` with
//!   `satisfied: false`. Reaching the deadline is itself a
//!   *conclusive* observation that the element / state never
//!   appeared.
//! - `ui/assert-url`: `Outcome::Timeout`. A page that never lands
//!   on the expected URL is not "we observed the wrong URL" — it's
//!   "we never got to where we said we would," which is the
//!   timeout-shaped outcome the judge maps to
//!   `Inconclusive(Timeout)`.
//!
//! Assertions over `$steps.<id>.outputs.satisfied` work in both
//! shapes; assertions that depend on a `pass` vs.
//! `inconclusive` verdict need to know which waiter they reference.

pub mod assert_element;
pub mod assert_state;
pub mod assert_url;
pub mod click;
pub mod navigate;
pub mod select;
pub mod type_;

pub use assert_element::AssertElement;
pub use assert_state::AssertState;
pub use assert_url::AssertUrl;
pub use click::Click;
pub use navigate::Navigate;
pub use select::Select;
pub use type_::Type;

/// Recognize the Playwright "operation timed out" error message
/// across actions. Playwright's Node driver doesn't differentiate
/// timeouts in its error type — only in the message — so we sniff.
pub(crate) fn is_timeout_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("timeout") || lower.contains("exceeded")
}
