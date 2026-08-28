//! `ui/extract` — read a concrete DOM value without judging it.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::action::{Action, ActionCtx, ActionResult};
use crate::error::ActionError;
use crate::locator::Locator;
use crate::playwright::to_selector;

use super::field::FieldSource;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    locator: Locator,
    #[serde(flatten)]
    source: FieldSource,
    #[serde(default)]
    all: bool,
}

pub struct Extract;

#[async_trait]
impl Action for Extract {
    fn uses(&self) -> &'static str {
        "ui/extract"
    }

    fn contract(&self) -> crate::action::ActionContract {
        use crate::action::{ActionContract, FieldSpec};
        ActionContract {
            uses: "ui/extract",
            summary: "Extract an attribute, DOM property, or rendered text from matching elements.",
            with: vec![
                FieldSpec::required("locator"),
                FieldSpec::optional("field"),
                FieldSpec::optional("attribute"),
                FieldSpec::optional("property"),
                FieldSpec::optional("text"),
                FieldSpec::optional("all"),
            ],
            outputs: vec!["value", "found", "count", "values"],
            secret_outputs: vec![],
            example: "- id: heading\n  uses: ui/extract\n  with: { locator: { css: h1 }, field: text }",
        }
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let page = ctx.require_page()?;
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|source| ActionError::InvalidWith {
                action: "ui/extract",
                source,
            })?;
        let selector = to_selector(&with.locator);
        let values = page
            .extract(&selector, with.source.kind(), with.source.name())
            .await
            .map_err(|e| ActionError::Playwright(format!("ui/extract: {e}")))?;
        let count = values.len();
        if !with.all && count > 1 {
            return Err(ActionError::Playwright(format!(
                "ui/extract: locator {} matched {count} elements; expected exactly one (set `all: true` to extract every match)",
                with.locator.describe()
            )));
        }
        let mut result = ActionResult::ok()
            .with_output("found", json!(count > 0))
            .with_output("count", json!(count));
        if with.all {
            result = result.with_output("values", json!(values));
        } else {
            result = result.with_output(
                "value",
                values.into_iter().next().unwrap_or(serde_json::Value::Null),
            );
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract_is_non_judging_and_source_validation_is_shared() {
        assert!(!Extract.contract().judges());
        assert!(serde_yml::from_str::<With>("locator: { css: h1 }").is_err());
        assert!(
            serde_yml::from_str::<With>("locator: { css: h1 }\nfield: text\nattribute: title")
                .is_err()
        );
        assert!(
            serde_yml::from_str::<With>("locator: { css: h1 }\nfield: unknown")
                .unwrap_err()
                .to_string()
                .contains("attribute:")
        );
    }
}
