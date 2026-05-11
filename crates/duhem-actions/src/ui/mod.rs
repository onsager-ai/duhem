//! `ui/*` actions — the Playwright-backed half of the v1 catalog.
//!
//! Each action is a unit struct implementing [`crate::Action`] and a
//! private `With` struct that types `Step.with` at the per-action
//! schema. The trait + lifecycle are deliberately the only thing the
//! runtime knows about; adding `ui/type`, `ui/select`, `ui/assert-url`,
//! `ui/assert-state` (deferred per spec) is more files in this
//! module — no trait change.

pub mod assert_element;
pub mod click;
pub mod navigate;

pub use assert_element::AssertElement;
pub use click::Click;
pub use navigate::Navigate;

/// Recognize the Playwright "operation timed out" error message
/// across actions. Playwright's Node driver doesn't differentiate
/// timeouts in its error type — only in the message — so we sniff.
pub(crate) fn is_timeout_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("timeout") || lower.contains("exceeded")
}
