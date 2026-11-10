//! `ui/press` — press a key or chord (Playwright key vocabulary).
//!
//! With a `locator` the action uses `locator.press`: it waits for
//! actionability, focuses the element and presses. Without one the key
//! goes through `page.keyboard.press` to whatever is focused. Either
//! way the `target` output records the receiver (`{tag, id, role,
//! name}`) sampled before dispatch, so a VD can assert who actually got
//! the key. `name` is an approximation of the accessible name
//! (aria-label, then aria-labelledby text, then trimmed text content
//! capped at 200 chars); `role` is the explicit role attribute or the
//! implicit role of common tags, else null.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Deserializer};

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_TIMEOUT};
use crate::error::ActionError;
use crate::locator::Locator;
use crate::playwright::to_selector;
use crate::with::TimeoutSpec;

fn non_empty_key<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let key = String::deserialize(deserializer)?;
    if key.is_empty() {
        return Err(serde::de::Error::custom(
            "`key` must not be empty; use a Playwright key such as Enter, Tab, ArrowDown or Shift+Tab",
        ));
    }
    Ok(key)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    #[serde(deserialize_with = "non_empty_key")]
    key: String,
    #[serde(default)]
    locator: Option<Locator>,
    #[serde(default)]
    timeout: Option<TimeoutSpec>,
}

pub struct Press;

#[async_trait]
impl Action for Press {
    fn uses(&self) -> &'static str {
        "ui/press"
    }

    fn contract(&self) -> crate::action::ActionContract {
        use crate::action::{ActionContract, FieldSpec};
        ActionContract {
            uses: "ui/press",
            summary: "Press a key or `+` chord (Playwright key names, e.g. Enter, Tab, Shift+Tab). With a locator the element is focused first; without one the key goes to the focused element. `target` records the receiver ({tag, id, role, name}, sampled before the press; `name` is an approximation of the accessible name).",
            with: vec![
                FieldSpec::required("key"),
                FieldSpec::optional("locator"),
                FieldSpec::optional("timeout"),
            ],
            outputs: vec!["target"],
            secret_outputs: vec![],
            example: "- id: open\n  uses: ui/press\n  with: { key: Enter }",
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
                action: "ui/press",
                source: e,
            })?;
        let timeout: Duration = with.timeout.map(Into::into).unwrap_or(DEFAULT_TIMEOUT);
        let timeout_ms = timeout.as_millis() as f64;

        let result = match &with.locator {
            Some(locator) => {
                let selector = to_selector(locator);
                page.for_locator(locator)
                    .press(Some(selector.as_str()), &with.key, timeout_ms)
                    .await
            }
            None => page.press(None, &with.key, timeout_ms).await,
        };

        match result {
            Ok(target) => Ok(ActionResult::ok().with_output("target", target)),
            Err(e) if super::is_timeout_message(&e.to_string()) => Ok(ActionResult::timeout()),
            Err(e) => Err(ActionError::Playwright(format!("ui/press: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_page_level_press() {
        let v: With = serde_yml::from_str("key: Enter").unwrap();
        assert_eq!(v.key, "Enter");
        assert!(v.locator.is_none());
        assert!(v.timeout.is_none());
    }

    #[test]
    fn parses_chord_with_locator_and_timeout() {
        let yaml = "{ key: Shift+Tab, locator: { role: button, name: Save }, timeout: 2s }";
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert_eq!(v.key, "Shift+Tab");
        assert_eq!(v.locator.unwrap().role.as_deref(), Some("button"));
        let d: Duration = v.timeout.unwrap().into();
        assert_eq!(d, Duration::from_secs(2));
    }

    #[test]
    fn rejects_empty_key_missing_key_and_unknown_field() {
        assert!(serde_yml::from_str::<With>("key: ''").is_err());
        assert!(serde_yml::from_str::<With>("{ locator: { role: button } }").is_err());
        assert!(serde_yml::from_str::<With>("{ key: Enter, repeat: 2 }").is_err());
    }
}
