//! `ui/wait` — a page-free fixed delay for debugging and transitional VDs.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionContract, ActionCtx, ActionResult, FieldSpec};
use crate::error::ActionError;
use crate::with::TimeoutSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    duration: TimeoutSpec,
}

pub struct Wait;

#[async_trait]
impl Action for Wait {
    fn uses(&self) -> &'static str {
        "ui/wait"
    }

    fn contract(&self) -> ActionContract {
        ActionContract {
            uses: "ui/wait",
            summary: "Wait for a fixed duration within the suite ceiling (debugging escape hatch; prefer an assertion timeout).",
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
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|source| ActionError::InvalidWith {
                action: "ui/wait",
                source,
            })?;
        let duration = Duration::from(with.duration);
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
}
