//! `ui/wait` — a page-free fixed delay for debugging and transitional VDs.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde::de::Error as _;

use crate::action::{Action, ActionContract, ActionCtx, ActionResult, FieldSpec};
use crate::error::ActionError;
use crate::with::TimeoutSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    duration: TimeoutSpec,
}

pub struct Wait;

const MAX_DURATION: Duration = Duration::from_secs(60);

#[async_trait]
impl Action for Wait {
    fn uses(&self) -> &'static str {
        "ui/wait"
    }

    fn contract(&self) -> ActionContract {
        ActionContract {
            uses: "ui/wait",
            summary: "Wait up to 60s for a fixed duration (debugging escape hatch; prefer an assertion timeout).",
            with: vec![FieldSpec::required("duration")],
            outputs: vec![],
            secret_outputs: vec![],
            example: "- uses: ui/wait\n  with: { duration: 500ms }",
        }
    }

    fn requires_page(&self) -> bool {
        false
    }

    async fn invoke(
        &self,
        _ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let duration_label = with
            .as_mapping()
            .and_then(|mapping| mapping.get(serde_yml::Value::String("duration".into())))
            .map(|value| match value {
                serde_yml::Value::String(raw) => raw.clone(),
                _ => serde_yml::to_string(value)
                    .unwrap_or_else(|_| "<unprintable>".into())
                    .trim()
                    .to_owned(),
            });
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|source| ActionError::InvalidWith {
                action: "ui/wait",
                source,
            })?;
        let duration = Duration::from(with.duration);
        if duration > MAX_DURATION {
            return Err(ActionError::InvalidWith {
                action: "ui/wait",
                source: serde_yml::Error::custom(format!(
                    "ui/wait duration `{}` exceeds the 60s maximum; use `ui/assert-element` with `timeout:` to wait for a condition",
                    duration_label.unwrap_or_else(|| format!("{}ms", duration.as_millis()))
                )),
            });
        }
        tokio::time::sleep(duration).await;
        Ok(ActionResult::ok())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::{Action, ActionCtx, Outcome};

    #[tokio::test]
    async fn parses_common_durations_and_waits_without_a_page() {
        let action = Wait;
        let ctx = ActionCtx {
            page: None,
            step_index: 0,
        };
        for raw in ["2s", "500ms"] {
            let with = serde_yml::from_str(&format!("duration: {raw}")).unwrap();
            let parsed: With = serde_yml::from_value(with).unwrap();
            assert!(Duration::from(parsed.duration) > Duration::ZERO);
        }

        let with = serde_yml::from_str("duration: 20ms").unwrap();
        let started = Instant::now();
        let result = action.invoke(&ctx, &with).await.unwrap();
        assert_eq!(result.outcome, Outcome::Ok);
        assert!(started.elapsed() >= Duration::from_millis(15));
        assert!(!action.requires_page());
        assert!(!action.contract().judges());
    }

    #[tokio::test]
    async fn duration_cap_is_inclusive_and_rejects_before_sleeping() {
        let action = Wait;
        let ctx = ActionCtx {
            page: None,
            step_index: 0,
        };

        let with = serde_yml::from_str("duration: 60s").unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), action.invoke(&ctx, &with))
                .await
                .is_err(),
            "60s must pass validation and begin sleeping"
        );

        for raw in ["61s", "2m"] {
            let with = serde_yml::from_str(&format!("duration: {raw}")).unwrap();
            let started = Instant::now();
            let error = action.invoke(&ctx, &with).await.unwrap_err();
            assert!(started.elapsed() < Duration::from_secs(1));
            let message = error.to_string();
            assert!(message.contains(&format!("duration `{raw}`")), "{message}");
            assert!(message.contains("60s maximum"), "{message}");
            assert!(message.contains("ui/assert-element"), "{message}");
        }
    }
}
