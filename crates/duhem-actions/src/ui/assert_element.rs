//! `ui/assert-element` — observe an element's existence/visibility.
//!
//! The action is a *waiter*: it polls for the element to reach the
//! requested `expected` state until the deadline, then reports what
//! it saw. Reaching the state within the deadline → `satisfied: true`;
//! deadline elapses → `satisfied: false` and `outcome: Ok` (the
//! observation is conclusive — the element wasn't there). Driver
//! errors that aren't timeout-shaped surface as `ActionError`.
//!
//! Note that `Outcome::Timeout` is reserved for the case where the
//! browser itself times out (page hung, navigation never completes).
//! A successful "we waited and it never appeared" observation is an
//! `Ok` with `satisfied: false` — that's the whole point of expecting
//! `not_exists` / `hidden`.

use std::time::Duration;

use async_trait::async_trait;
use playwright::api::frame::FrameState;
use serde::Deserialize;
use serde_json::json;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::error::ActionError;
use crate::locator::{ExistenceState, Locator};
use crate::playwright::to_selector;
use crate::with::WithinSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    locator: Locator,
    expected: ExistenceState,
    #[serde(default)]
    within: Option<WithinSpec>,
}

pub struct AssertElement;

#[async_trait]
impl Action for AssertElement {
    fn uses(&self) -> &'static str {
        "ui/assert-element"
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "ui/assert-element",
                source: e,
            })?;
        let timeout: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);
        let selector = to_selector(&with.locator);

        let wait = ctx
            .page
            .wait_for_selector_builder(&selector)
            .state(map_state(with.expected))
            .timeout(timeout.as_millis() as f64)
            .wait_for_selector();

        let satisfied = match wait.await {
            Ok(Some(_)) => true,
            Ok(None) => true, // `state: Detached` returns Ok(None) on success.
            Err(e) if super::is_timeout_message(&e.to_string()) => false,
            Err(e) => return Err(ActionError::Playwright(format!("ui/assert-element: {e}"))),
        };

        // `count` reflects matches at observation time. Even when the
        // expectation is `not_exists`/`hidden`, this is the literal
        // post-wait DOM count.
        let count = match ctx.page.query_selector_all(&selector).await {
            Ok(v) => v.len() as u32,
            Err(_) => 0,
        };

        Ok(ActionResult::ok()
            .with_output("satisfied", json!(satisfied))
            .with_output("count", json!(count)))
    }
}

fn map_state(s: ExistenceState) -> FrameState {
    match s {
        ExistenceState::Exists => FrameState::Attached,
        ExistenceState::NotExists => FrameState::Detached,
        ExistenceState::Visible => FrameState::Visible,
        ExistenceState::Hidden => FrameState::Hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assert_element_with() {
        let yaml = r#"
locator: { role: alert, text: "Created" }
expected: visible
within: 2s
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert_eq!(v.expected, ExistenceState::Visible);
        let d: Duration = v.within.unwrap().into();
        assert_eq!(d, Duration::from_secs(2));
        assert_eq!(v.locator.text.as_deref(), Some("Created"));
    }

    #[test]
    fn rejects_unknown_expected() {
        let yaml = r#"{ locator: { role: alert }, expected: kind_of_there }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn requires_locator() {
        let yaml = r#"{ expected: visible }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }
}
