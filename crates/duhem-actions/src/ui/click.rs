//! `ui/click` — click an element addressed by `getByRole`-style fields.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_TIMEOUT};
use crate::error::ActionError;
use crate::locator::Locator;
use crate::playwright::to_selector;
use crate::with::TimeoutSpec;

// The locator fields sit inline in `ui/click`'s `with:` (`{ role: button,
// name: Create, timeout: 3s }`), not under a `locator:` key — kept that way
// for backward compatibility. `WithWire` collects the inline fields
// (rejecting unknowns), then folds them into a validated `Locator` so click
// gains label/testid/css/placeholder and the exactly-one-primary check.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WithWire {
    #[serde(default)]
    locator: Option<Locator>,
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
    timeout: Option<TimeoutSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "WithWire")]
struct With {
    locator: Locator,
    timeout: Option<TimeoutSpec>,
}

impl TryFrom<WithWire> for With {
    type Error = String;

    fn try_from(w: WithWire) -> Result<Self, Self::Error> {
        let inline = Locator {
            role: w.role,
            label: w.label,
            testid: w.testid,
            placeholder: w.placeholder,
            css: w.css,
            name: w.name,
            text: w.text,
            scope: w.scope,
        };
        let has_inline = inline.role.is_some()
            || inline.label.is_some()
            || inline.testid.is_some()
            || inline.placeholder.is_some()
            || inline.css.is_some()
            || inline.name.is_some()
            || inline.text.is_some()
            || inline.scope.is_some();
        let locator = match (w.locator, has_inline) {
            (Some(_), true) => {
                return Err(
                    "use either `locator:` or inline locator fields on ui/click, not both"
                        .to_string(),
                );
            }
            (Some(locator), false) => locator,
            (None, _) => inline,
        };
        locator.validate_primary()?;
        Ok(With {
            locator,
            timeout: w.timeout,
        })
    }
}

impl With {
    fn into_locator(self) -> (Locator, Duration) {
        let timeout = self.timeout.map(Into::into).unwrap_or(DEFAULT_TIMEOUT);
        (self.locator, timeout)
    }
}

pub struct Click;

#[async_trait]
impl Action for Click {
    fn uses(&self) -> &'static str {
        "ui/click"
    }

    fn contract(&self) -> crate::action::ActionContract {
        use crate::action::{ActionContract, FieldSpec};
        ActionContract {
            uses: "ui/click",
            summary: "Click an element (locator shorthand fields, or a `locator` object).",
            with: vec![
                FieldSpec::optional("role"),
                FieldSpec::optional("label"),
                FieldSpec::optional("testid"),
                FieldSpec::optional("placeholder"),
                FieldSpec::optional("css"),
                FieldSpec::optional("name"),
                FieldSpec::optional("text"),
                FieldSpec::optional("scope"),
                FieldSpec::optional("locator"),
                FieldSpec::optional("timeout"),
            ],
            outputs: vec![],
            secret_outputs: vec![],
            example: "- uses: ui/click\n  with: { locator: { role: button, name: \"Save\" } }",
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
timeout: 3s
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        let (l, t) = v.into_locator();
        assert_eq!(l.scope.as_ref().unwrap().role.as_deref(), Some("list"));
        assert_eq!(t, Duration::from_secs(3));
    }

    #[test]
    fn parses_nested_locator_for_catalog_splicing() {
        let yaml = r#"{ locator: { role: button, name: Create }, timeout: 3s }"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        let (locator, timeout) = v.into_locator();
        assert_eq!(locator.role.as_deref(), Some("button"));
        assert_eq!(locator.name.as_deref(), Some("Create"));
        assert_eq!(timeout, Duration::from_secs(3));
    }

    #[test]
    fn rejects_nested_and_inline_locator_together() {
        let yaml = r#"{ locator: { role: button }, role: button }"#;
        let error = serde_yml::from_str::<With>(yaml).unwrap_err().to_string();
        assert!(error.contains("either `locator:` or inline"), "{error}");
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = r#"{ role: button, color: red }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }
}
