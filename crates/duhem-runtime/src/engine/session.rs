//! Per-check browser-session resolution (spec #347).
//!
//! A `session:` expression is evaluated before the browser context is
//! created, after run-level setup outputs exist. The result is copied
//! into a fresh context; contexts are never shared. Resolution failure
//! is deliberately data-plane failure (`Inconclusive(EnvironmentError)`)
//! rather than an engine error, matching browser context allocation.

use duhem_schema::{Check, Expr};
use sha2::{Digest, Sha256};

use crate::engine::context::{RunContext, RunState, value_to_json};
use crate::eval::eval_to_value;

/// The runtime result of interpreting one check's `session:` field.
pub(crate) struct SessionResolution {
    /// Literal authored reference. Absent when no UI step consumes a
    /// session, even if a page-free check carries the advisory field.
    pub source: Option<String>,
    /// Resolved Playwright storage state plus its credential-free
    /// evidence digest.
    pub seed: Option<SessionSeed>,
    /// Parse, lookup, conversion, or browser-seed preparation failed.
    pub failed: bool,
}

pub(crate) struct SessionSeed {
    pub state: serde_json::Value,
    pub digest: String,
}

impl SessionResolution {
    pub fn digest(&self) -> Option<String> {
        self.seed.as_ref().map(|seed| seed.digest.clone())
    }

    /// Open one new context from this resolution. Centralizing the
    /// seeded/unseeded dispatch keeps the runner concerned only with
    /// environment-failure policy.
    pub async fn open_check(
        &self,
        browser: &duhem_actions::RunBrowser,
    ) -> Result<duhem_actions::CheckBrowser, duhem_actions::ActionError> {
        match self.seed.as_ref() {
            Some(seed) => browser.open_check_with_storage_state(&seed.state).await,
            None => browser.open_check().await,
        }
    }
}

/// Resolve only for checks containing a `ui/*` step. A `session:` on
/// an API/DB/CLI-only check is an authoring warning and an operational
/// no-op, so it cannot make an otherwise valid run fail.
pub(crate) fn resolve(check: &Check, run: &RunState) -> SessionResolution {
    let consumes_session = check
        .steps
        .iter()
        .any(|step| step.uses_name().starts_with("ui/"));
    let Some(source) = check.session.as_ref().filter(|_| consumes_session) else {
        return SessionResolution {
            source: None,
            seed: None,
            failed: false,
        };
    };

    let resolved = duhem_schema::expr::parse(source)
        .ok()
        .and_then(|expr| match expr {
            Expr::Path(_) => eval_to_value(&expr, &RunContext::new(run)).ok(),
            _ => None,
        })
        .map(|value| value_to_json(&value));

    match resolved {
        Some(state) => {
            let bytes = serde_json::to_vec(&state).expect("runtime value serializes to JSON");
            let digest = hex::encode(Sha256::digest(bytes));
            SessionResolution {
                source: Some(source.clone()),
                seed: Some(SessionSeed { state, digest }),
                failed: false,
            }
        }
        None => SessionResolution {
            source: Some(source.clone()),
            seed: None,
            failed: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use duhem_schema::VerificationDefinition;

    use super::*;
    use crate::engine::context::{RunState, json_to_value};

    fn check(yaml: &str) -> Check {
        VerificationDefinition::from_yaml_str(yaml)
            .unwrap()
            .criteria[0]
            .checks[0]
            .clone()
    }

    #[test]
    fn resolves_input_object_and_hashes_canonical_json() {
        let state = serde_json::json!({"origins": [], "cookies": []});
        let mut inputs = BTreeMap::new();
        inputs.insert("state".into(), json_to_value(&state).unwrap());
        let run = RunState::new(inputs);
        let check = check(
            r#"
verification: x
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        session: $inputs.state
        steps:
          - uses: ui/navigate
            with: { url: about:blank }
        assertions: ["true"]
"#,
        );
        let resolved = resolve(&check, &run);
        assert!(!resolved.failed);
        assert_eq!(resolved.seed.as_ref().unwrap().state, state);
        assert_eq!(
            resolved.seed.unwrap().digest,
            "dcbfcdab9989eddcd68fdfe131c719283e1960b866b600a3d36d6daff254f32b"
        );
    }

    #[test]
    fn page_free_session_is_not_resolved() {
        let run = RunState::new(BTreeMap::new());
        let check = check(
            r#"
verification: x
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        session: $inputs.missing
        steps:
          - uses: cli/invoke
            with: { command: [true] }
        assertions: ["true"]
"#,
        );
        let resolved = resolve(&check, &run);
        assert!(!resolved.failed);
        assert!(resolved.source.is_none());
        assert!(resolved.seed.is_none());
    }
}
