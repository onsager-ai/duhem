//! `ui/click` — click an element addressed by `getByRole`-style fields.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::error::ActionError;
use crate::locator::Locator;
use crate::playwright::to_selector;
use crate::with::WithinSpec;

// The locator fields sit inline in `ui/click`'s `with:` (`{ role: button,
// name: Create, within: 3s }`), not under a `locator:` key — kept that way
// for backward compatibility. `WithWire` collects the inline fields
// (rejecting unknowns), then folds them into a validated `Locator` so click
// gains label/testid/css/placeholder and the exactly-one-primary check.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WithWire {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    testid: Option<String>,
    #[serde(default)]
    placeholder: Option<String>,
    #[serde(default)]
    css: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    scope: Option<Box<Locator>>,
    #[serde(default)]
    within: Option<WithinSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "WithWire")]
struct With {
    locator: Locator,
    within: Option<WithinSpec>,
}

impl TryFrom<WithWire> for With {
    type Error = String;

    fn try_from(w: WithWire) -> Result<Self, Self::Error> {
        let locator = Locator {
            role: w.role,
            label: w.label,
            testid: w.testid,
            placeholder: w.placeholder,
            css: w.css,
            name: w.name,
            text: w.text,
            scope: w.scope,
        };
        locator.validate_primary()?;
        Ok(With {
            locator,
            within: w.within,
        })
    }
}

impl With {
    fn into_locator(self) -> (Locator, Duration) {
        let timeout = self.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);
        (self.locator, timeout)
    }
}

pub struct Click;

#[async_trait]
impl Action for Click {
    fn uses(&self) -> &'static str {
        "ui/click"
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let page = ctx.require_page()?;
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "ui/click",
                source: e,
            })?;
        let (locator, timeout) = with.into_locator();
        let selector = to_selector(&locator);

        match page.click(&selector, timeout.as_millis() as f64).await {
            Ok(()) => Ok(ActionResult::ok()),
            Err(e) if super::is_timeout_message(&e.to_string()) => Ok(ActionResult::timeout()),
            Err(e) => Err(ActionError::Playwright(format!("ui/click: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_click() {
        let yaml = r#"{ role: button, name: Create }"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        let (l, _) = v.into_locator();
        assert_eq!(l.role.as_deref(), Some("button"));
        assert_eq!(l.name.as_deref(), Some("Create"));
    }

    #[test]
    fn parses_click_with_scope() {
        let yaml = r#"
role: button
name: Create
scope: { role: list, name: Workspaces }
within: 3s
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        let (l, t) = v.into_locator();
        assert_eq!(l.scope.as_ref().unwrap().role.as_deref(), Some("list"));
        assert_eq!(t, Duration::from_secs(3));
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = r#"{ role: button, color: red }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }
}
