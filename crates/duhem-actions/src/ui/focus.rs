//! `ui/focus` — move focus to an element with `locator.focus()`.
//!
//! This is *programmatic* focus (`element.focus()`), not a user
//! gesture: it does not exercise Tab order or pointer behavior. To
//! verify keyboard focus movement, use `ui/press { key: Tab }` and
//! then `ui/assert-element { expected: focused }`.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_TIMEOUT};
use crate::error::ActionError;
use crate::locator::Locator;
use crate::playwright::to_selector;
use crate::with::TimeoutSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    locator: Locator,
    #[serde(default)]
    timeout: Option<TimeoutSpec>,
}

pub struct Focus;

#[async_trait]
impl Action for Focus {
    fn uses(&self) -> &'static str {
        "ui/focus"
    }

    fn contract(&self) -> crate::action::ActionContract {
        use crate::action::{ActionContract, FieldSpec};
        ActionContract {
            uses: "ui/focus",
            summary: "Focus an element programmatically (element.focus(), not a user gesture). To verify keyboard focus movement, use ui/press with Tab and then ui/assert-element with expected: focused.",
            with: vec![FieldSpec::required("locator"), FieldSpec::optional("timeout")],
            outputs: vec![],
            secret_outputs: vec![],
            example: "- uses: ui/focus\n  with: { locator: { role: button, name: Options } }",
        }
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let page = ctx.require_page()?;
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "ui/focus",
                source: e,
            })?;
        let timeout: Duration = with.timeout.map(Into::into).unwrap_or(DEFAULT_TIMEOUT);
        let selector = to_selector(&with.locator);
        let page = page.for_locator(&with.locator);

        match page.focus(&selector, timeout.as_millis() as f64).await {
            Ok(()) => Ok(ActionResult::ok()),
            Err(e) if super::is_timeout_message(&e.to_string()) => Ok(ActionResult::timeout()),
            Err(e) => Err(ActionError::Playwright(format!("ui/focus: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_focus_with() {
        let yaml = "{ locator: { role: button, name: Options }, timeout: 1s }";
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert_eq!(v.locator.role.as_deref(), Some("button"));
        let d: Duration = v.timeout.unwrap().into();
        assert_eq!(d, Duration::from_secs(1));
    }

    #[test]
    fn requires_locator_and_rejects_unknown_field() {
        assert!(serde_yml::from_str::<With>("{ timeout: 1s }").is_err());
        assert!(serde_yml::from_str::<With>("{ locator: { role: button }, key: Tab }").is_err());
    }
}
