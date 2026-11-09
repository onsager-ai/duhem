//! `ui/back` / `ui/forward` — move through the page's real browser history.
//!
//! A missing history entry is not an error: the action reports
//! `navigated: false` with the URL unchanged. It never falls back to
//! direct navigation (#580 S2).

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::action::{Action, ActionContract, ActionCtx, ActionResult, DEFAULT_TIMEOUT, FieldSpec};
use crate::error::ActionError;
use crate::with::TimeoutSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    #[serde(default)]
    timeout: Option<TimeoutSpec>,
}

async fn move_in_history(
    action: &'static str,
    direction: &str,
    ctx: &ActionCtx<'_>,
    with: &serde_yml::Value,
) -> Result<ActionResult, ActionError> {
    let page = ctx.require_page()?;
    // `with:` may be omitted entirely (YAML null), so parse as optional.
    let with: Option<With> = serde_yml::from_value(with.clone())
        .map_err(|source| ActionError::InvalidWith { action, source })?;
    let timeout: Duration = with
        .and_then(|w| w.timeout)
        .map(Into::into)
        .unwrap_or(DEFAULT_TIMEOUT);

    match page.history(direction, timeout.as_millis() as f64).await {
        Ok((from_url, url, navigated)) => Ok(ActionResult::ok()
            .with_output("from_url", json!(from_url))
            .with_output("url", json!(url))
            .with_output("navigated", json!(navigated))),
        Err(e) if super::is_timeout_message(&e.to_string()) => Ok(ActionResult::timeout()),
        Err(e) => Err(ActionError::Playwright(format!("{action}: {e}"))),
    }
}

pub struct Back;

#[async_trait]
impl Action for Back {
    fn uses(&self) -> &'static str {
        "ui/back"
    }

    fn contract(&self) -> ActionContract {
        ActionContract {
            uses: "ui/back",
            summary: "Go back one real browser history entry (same-document entries included); `navigated` is false, with the URL unchanged, when no entry exists.",
            with: vec![FieldSpec::optional("timeout")],
            outputs: vec!["from_url", "url", "navigated"],
            secret_outputs: vec![],
            example: "- id: back\n  uses: ui/back",
        }
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        move_in_history("ui/back", "back", ctx, with).await
    }
}

pub struct Forward;

#[async_trait]
impl Action for Forward {
    fn uses(&self) -> &'static str {
        "ui/forward"
    }

    fn contract(&self) -> ActionContract {
        ActionContract {
            uses: "ui/forward",
            summary: "Go forward one real browser history entry (same-document entries included); `navigated` is false, with the URL unchanged, when no entry exists.",
            with: vec![FieldSpec::optional("timeout")],
            outputs: vec!["from_url", "url", "navigated"],
            secret_outputs: vec![],
            example: "- id: forward\n  uses: ui/forward",
        }
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        move_in_history("ui/forward", "forward", ctx, with).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_accepts_only_timeout() {
        let v: With = serde_yml::from_str("{ timeout: 2s }").unwrap();
        let d: Duration = v.timeout.unwrap().into();
        assert_eq!(d, Duration::from_secs(2));
        assert!(serde_yml::from_str::<With>("{}").unwrap().timeout.is_none());
        assert!(serde_yml::from_str::<With>("{ url: http://x }").is_err());
        let omitted: Option<With> = serde_yml::from_value(serde_yml::Value::Null).unwrap();
        assert!(omitted.is_none());
    }

    #[test]
    fn contracts_are_non_judging_and_report_the_history_facts() {
        for c in [Back.contract(), Forward.contract()] {
            assert!(!c.judges());
            assert_eq!(c.outputs, vec!["from_url", "url", "navigated"]);
        }
    }
}
