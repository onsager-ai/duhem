//! Per-check browser-session resolution (spec #347).
//!
//! A `session:` expression is evaluated before the browser context is
//! created, after run-level setup outputs exist. The result is copied
//! into a fresh context; contexts are never shared. Resolution failure
//! is deliberately data-plane failure (`Inconclusive(EnvironmentError)`)
//! rather than an engine error, matching browser context allocation.

use std::collections::BTreeMap;

use duhem_schema::{Check, Expr};
use sha2::{Digest, Sha256};

use crate::engine::context::{RunContext, RunState, value_to_json};
use crate::eval::eval_to_value;

/// The runtime result of interpreting one check's `session:` field.
#[derive(Debug, Clone)]
pub(crate) struct SessionResolution {
    /// Literal authored reference. Absent when no UI step consumes a
    /// session, even if a page-free check carries the advisory field.
    pub source: Option<String>,
    /// Resolved Playwright storage state plus its credential-free
    /// evidence digest.
    pub seed: Option<SessionSeed>,
    /// Parse, lookup, conversion, or browser-seed preparation failed.
    pub failed: bool,
    pub named: BTreeMap<String, SessionResolution>,
}

#[derive(Debug, Clone)]
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

pub(super) fn resolve_source(source: Option<&str>, run: &RunState) -> SessionResolution {
    let Some(source) = source else {
        return SessionResolution {
            source: None,
            seed: None,
            failed: false,
            named: BTreeMap::new(),
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
                source: Some(source.to_string()),
                seed: Some(SessionSeed { state, digest }),
                failed: false,
                named: BTreeMap::new(),
            }
        }
        None => SessionResolution {
            source: Some(source.to_string()),
            seed: None,
            failed: true,
            named: BTreeMap::new(),
        },
    }
}

/// Contexts and investigation evidence owned by one check attempt.
#[derive(Default)]
pub(crate) struct CheckContexts {
    pub browsers: BTreeMap<Option<String>, duhem_actions::CheckBrowser>,
    pub targets: BTreeMap<Option<String>, Vec<super::capture::TargetLocator>>,
    pub storyboards: BTreeMap<Option<String>, super::capture::Storyboard>,
    pub failed: bool,
    permits: Vec<super::session_scope::ContextPermit>,
}

impl CheckContexts {
    pub async fn open(
        session: &SessionResolution,
        browser: Option<&duhem_actions::RunBrowser>,
        budget: Option<&std::sync::Arc<super::session_scope::ContextBudget>>,
    ) -> Result<Self, super::outcome::EngineError> {
        let mut contexts = Self {
            failed: session.failed || browser.is_none(),
            ..Self::default()
        };
        if !contexts.failed
            && let Some(browser) = browser
        {
            let seeds: Vec<_> = if session.named.is_empty() {
                vec![(None, session)]
            } else {
                session
                    .named
                    .iter()
                    .map(|(name, seed)| (Some(name.clone()), seed))
                    .collect()
            };
            for (name, seed) in seeds {
                let permit = match budget.map(|budget| budget.reserve()).transpose() {
                    Ok(permit) => permit,
                    Err(error) => {
                        contexts.close().await;
                        return Err(error);
                    }
                };
                match seed.open_check(browser).await {
                    Ok(context) => {
                        contexts.browsers.insert(name, context);
                        contexts.permits.extend(permit);
                    }
                    Err(error) => {
                        tracing::debug!(%error, "context allocation failed");
                        contexts.failed = true;
                        break;
                    }
                }
            }
        }
        Ok(contexts)
    }

    pub async fn close(&mut self) {
        for (_, browser) in std::mem::take(&mut self.browsers) {
            let _ = browser.close(false, 0).await;
        }
        self.permits.clear();
    }

    pub async fn finish(
        &mut self,
        writer: &mut duhem_evidence::EvidenceWriter,
        capture: bool,
        check: &Check,
    ) -> Vec<super::outcome::CapturedArtifact> {
        let mut artifacts = Vec::new();
        for (name, browser) in std::mem::take(&mut self.browsers) {
            writer.set_session(name.as_deref());
            let last = check
                .steps
                .iter()
                .rposition(|step| step.session == name)
                .unwrap_or(0) as u32;
            artifacts.extend(
                super::capture::finalize_capture(
                    writer,
                    browser,
                    capture,
                    last,
                    &self.targets.remove(&name).unwrap_or_default(),
                    self.storyboards.remove(&name).unwrap_or_default(),
                )
                .await,
            );
        }
        self.permits.clear();
        writer.set_session(None);
        artifacts
    }
}

/// Emit only references and digests for contexts that actually opened.
pub(super) fn lifecycle_evidence(
    session: &SessionResolution,
    contexts: &CheckContexts,
) -> (serde_json::Value, serde_json::Value) {
    if session.named.is_empty() {
        return (
            serde_json::json!(session.source),
            serde_json::json!(session.digest()),
        );
    }
    let mut sources = serde_json::Map::new();
    let mut digests = serde_json::Map::new();
    for name in contexts.browsers.keys().flatten() {
        let seed = &session.named[name];
        sources.insert(name.clone(), serde_json::json!(seed.source));
        digests.insert(name.clone(), serde_json::json!(seed.digest()));
    }
    (sources.into(), digests.into())
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
        let mut run = RunState::new(inputs);
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
        super::super::session_scope::SessionScope::enter(
            &mut run,
            &check.session,
            check.sessions.as_ref(),
        );
        let resolved = super::super::session_scope::SessionScope::resolve(&run);
        assert!(!resolved.failed);
        assert_eq!(resolved.seed.as_ref().unwrap().state, state);
        assert_eq!(
            resolved.seed.unwrap().digest,
            "dcbfcdab9989eddcd68fdfe131c719283e1960b866b600a3d36d6daff254f32b"
        );
    }

    #[test]
    fn page_free_session_is_not_resolved() {
        let mut run = RunState::new(BTreeMap::new());
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
        super::super::session_scope::SessionScope::enter(
            &mut run,
            &check.session,
            check.sessions.as_ref(),
        );
        let resolved = super::super::session_scope::SessionScope::observed(&run);
        assert!(!resolved.failed);
        assert!(resolved.source.is_none());
        assert!(resolved.seed.is_none());
    }
}
