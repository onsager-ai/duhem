//! `ui/type` — type into an input addressed by a `Locator`.
//!
//! `clear: true` (default) replaces the existing value via
//! Playwright's `Locator.fill`. `clear: false` appends via
//! `Locator.type`. The author-intuition default — "type 'Alice' into
//! the name field" usually meaning *replace* — is the Alignment
//! decision on issue #37.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::error::ActionError;
use crate::locator::Locator;
use crate::playwright::to_selector;
use crate::with::WithinSpec;

fn default_clear() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    locator: Locator,
    text: String,
    #[serde(default = "default_clear")]
    clear: bool,
    #[serde(default)]
    within: Option<WithinSpec>,
}

pub struct Type;

#[async_trait]
impl Action for Type {
    fn uses(&self) -> &'static str {
        "ui/type"
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "ui/type",
                source: e,
            })?;
        let timeout: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);
        let selector = to_selector(&with.locator);
        let timeout_ms = timeout.as_millis() as f64;

        let result = if with.clear {
            ctx.page
                .fill_builder(&selector, &with.text)
                .timeout(timeout_ms)
                .fill()
                .await
        } else {
            // `type_builer` — sic. That's how the upstream `playwright`
            // crate (0.0.20) names the builder; do not "fix" the typo
            // here without bumping the crate.
            ctx.page
                .type_builer(&selector, &with.text)
                .timeout(timeout_ms)
                .r#type()
                .await
        };

        match result {
            Ok(()) => Ok(ActionResult::ok()),
            Err(e) if super::is_timeout_message(&e.to_string()) => Ok(ActionResult::timeout()),
            Err(e) => Err(ActionError::Playwright(format!("ui/type: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_type() {
        let yaml = r#"
locator: { role: textbox, name: Name }
text: Alice
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert_eq!(v.locator.role, "textbox");
        assert_eq!(v.text, "Alice");
        assert!(v.clear, "clear defaults to true");
        assert!(v.within.is_none());
    }

    #[test]
    fn parses_type_append() {
        let yaml = r#"
locator: { role: textbox, name: Name }
text: " Smith"
clear: false
within: 2s
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert!(!v.clear);
        let d: Duration = v.within.unwrap().into();
        assert_eq!(d, Duration::from_secs(2));
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = r#"{ locator: { role: textbox }, text: x, mode: fast }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn requires_locator_and_text() {
        let without_locator = r#"{ text: x }"#;
        assert!(serde_yml::from_str::<With>(without_locator).is_err());
        let without_text = r#"{ locator: { role: textbox } }"#;
        assert!(serde_yml::from_str::<With>(without_text).is_err());
    }
}
