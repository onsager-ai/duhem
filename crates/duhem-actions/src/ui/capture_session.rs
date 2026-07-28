//! `ui/capture-session` — make ambient browser authentication state a
//! run-scoped value (spec #347).
//!
//! Setup and checks intentionally use different browser contexts, so
//! cookies/local storage cannot cross that boundary by accident. This
//! adapter captures Playwright `storageState` as an opaque output that
//! a later check may explicitly select through `session:`. Its contract
//! marks the output secret: credentials remain protected without every
//! Verification Definition repeating a `secret:` declaration.

use async_trait::async_trait;

use crate::action::{Action, ActionCtx, ActionResult};
use crate::error::ActionError;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct With {}

pub struct CaptureSession;

#[async_trait]
impl Action for CaptureSession {
    fn uses(&self) -> &'static str {
        "ui/capture-session"
    }

    fn contract(&self) -> crate::action::ActionContract {
        use crate::action::ActionContract;
        ActionContract {
            uses: "ui/capture-session",
            summary: "Capture the current browser context storage state for a later check.",
            with: vec![],
            outputs: vec!["state"],
            secret_outputs: vec!["state"],
            example: "- id: session\n  uses: ui/capture-session",
        }
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let _: Option<With> =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "ui/capture-session",
                source: e,
            })?;
        let state = ctx
            .require_page()?
            .storage_state()
            .await
            .map_err(|e| ActionError::Playwright(format!("ui/capture-session: {e}")))?;
        Ok(ActionResult::ok().with_output("state", state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_declares_state_secret_without_authored_help() {
        let contract = CaptureSession.contract();
        assert!(contract.with.is_empty());
        assert_eq!(contract.outputs, vec!["state"]);
        assert_eq!(contract.secret_outputs, vec!["state"]);
        assert!(!contract.judges());
    }

    #[test]
    fn with_is_closed_and_empty() {
        let empty: With = serde_yml::from_str("{}").expect("empty with");
        let _ = empty;
        let omitted: Option<With> = serde_yml::from_value(serde_yml::Value::Null).unwrap();
        assert!(omitted.is_none());
        assert!(serde_yml::from_str::<With>("{ extra: true }").is_err());
    }
}
