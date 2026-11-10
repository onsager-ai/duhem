//! Windowed page-error and console observations.

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionContract, ActionCtx, ActionResult, FieldSpec};
use crate::error::ActionError;
use crate::observe_window::{Window, resolve};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    window: Window,
}

pub struct ObserveConsole;

#[async_trait]
impl Action for ObserveConsole {
    fn uses(&self) -> &'static str {
        "ui/observe-console"
    }

    fn contract(&self) -> ActionContract {
        ActionContract {
            uses: "ui/observe-console",
            summary: "Observe page errors and console messages in a step-anchored window.",
            with: vec![FieldSpec::required("window")],
            outputs: vec![
                "page_errors",
                "page_error_count",
                "console",
                "console_count",
                "console_error_count",
                "window",
                "complete",
                "truncated",
            ],
            secret_outputs: vec![],
            example: "- uses: ui/observe-console\n  with:\n    window: { from_step: load, edge: start, duration: 1s }",
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
                action: "ui/observe-console",
                source,
            })?;
        let (edge, start_ms, end_ms) = resolve(&with.window, ctx, page, self.uses()).await?;
        let batch = page
            .console_records()
            .await
            .map_err(|e| ActionError::Playwright(format!("ui/observe-console: recorder: {e}")))?;
        let events = batch
            .get("events")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                ActionError::Playwright("ui/observe-console: invalid recorder events".into())
            })?;
        let attached_ms = batch
            .get("attached_ms")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| {
                ActionError::Playwright(
                    "ui/observe-console: invalid recorder attachment time".into(),
                )
            })?;
        let truncated = batch
            .get("truncated")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                ActionError::Playwright(
                    "ui/observe-console: invalid recorder truncation state".into(),
                )
            })?;
        let mut page_errors = Vec::new();
        let mut console = Vec::new();
        for event in events {
            let at = event
                .get("at_ms")
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| {
                    ActionError::Playwright("ui/observe-console: invalid event time".into())
                })?;
            if at < start_ms || at > end_ms {
                continue;
            }
            let mut event = event.clone();
            let kind = event.as_object_mut().and_then(|obj| obj.remove("kind"));
            match kind.as_ref().and_then(serde_json::Value::as_str) {
                Some("page_error") => page_errors.push(event),
                Some("console") => console.push(event),
                _ => {
                    return Err(ActionError::Playwright(
                        "ui/observe-console: invalid event kind".into(),
                    ));
                }
            }
        }
        let error_count = console
            .iter()
            .filter(|event| event["level"] == "error")
            .count();
        Ok(ActionResult::ok()
            .with_output("page_error_count", serde_json::json!(page_errors.len()))
            .with_output("page_errors", serde_json::json!(page_errors))
            .with_output("console_count", serde_json::json!(console.len()))
            .with_output("console_error_count", serde_json::json!(error_count))
            .with_output("console", serde_json::json!(console))
            .with_output("window", serde_json::json!({"from_step": with.window.from_step, "edge": edge, "start_ms": start_ms, "end_ms": end_ms}))
            .with_output("complete", serde_json::json!(attached_ms < start_ms && !truncated))
            .with_output("truncated", serde_json::json!(truncated)))
    }
}

#[cfg(test)]
mod tests {
    use super::With;

    #[test]
    fn window_is_required_and_validated() {
        assert!(serde_yml::from_str::<With>("{}").is_err());
        assert!(serde_yml::from_str::<With>("window: { from_step: load, duration: 0ms }").is_err());
        assert!(
            serde_yml::from_str::<With>("window: { from_step: load, edge: other, duration: 1s }")
                .is_err()
        );
        assert!(serde_yml::from_str::<With>("window: { from_step: load, duration: 1s }").is_ok());
    }
}
