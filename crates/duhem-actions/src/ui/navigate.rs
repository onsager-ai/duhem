//! `ui/navigate` — drive the browser to a URL.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::error::ActionError;
use crate::with::WithinSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    url: String,
    #[serde(default)]
    within: Option<WithinSpec>,
}

pub struct Navigate;

#[async_trait]
impl Action for Navigate {
    fn uses(&self) -> &'static str {
        "ui/navigate"
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let page = ctx.require_page()?;
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "ui/navigate",
                source: e,
            })?;
        let timeout: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);

        match page.goto(&with.url, timeout.as_millis() as f64).await {
            Ok(()) => Ok(ActionResult::ok()),
            Err(e) if super::is_timeout_message(&e.to_string()) => Ok(ActionResult::timeout()),
            Err(e) => Err(ActionError::Playwright(format!("ui/navigate: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_navigate_with() {
        let yaml = r#"{ url: "http://localhost:8080/", within: 2s }"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert_eq!(v.url, "http://localhost:8080/");
        let d: Duration = v.within.unwrap().into();
        assert_eq!(d, Duration::from_secs(2));
    }

    #[test]
    fn parses_navigate_without_within() {
        let yaml = r#"{ url: "http://x" }"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert!(v.within.is_none());
    }
}
