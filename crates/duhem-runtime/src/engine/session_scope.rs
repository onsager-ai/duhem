//! Lazy immutable seeds and a shared simultaneous-context ceiling.
use std::collections::BTreeMap;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};

use super::context::RunState;
use super::outcome::EngineError;
use super::session::{SessionResolution, resolve_source};

#[derive(Debug)]
pub(crate) struct SessionScope {
    sources: BTreeMap<Option<String>, Option<String>>,
    baseline: RunState,
    resolved: OnceLock<SessionResolution>,
}

impl SessionScope {
    pub fn enter(
        run: &mut RunState,
        scalar: &Option<Option<String>>,
        named: Option<&BTreeMap<String, Option<duhem_schema::ExprStr>>>,
    ) {
        if scalar.is_none() && named.is_none() {
            return;
        }
        let sources = match named {
            Some(named) => named
                .iter()
                .map(|(name, expr)| (Some(name.clone()), expr.as_ref().map(|e| e.raw.clone())))
                .collect(),
            None => BTreeMap::from([(None, scalar.clone().flatten())]),
        };
        // Capture only enclosing state. Resolution is delayed until a browser
        // block consumes it, and cached across children and retry attempts.
        let mut baseline = RunState::new(run.inputs.clone());
        baseline.setup_outputs = run.setup_outputs.clone();
        baseline.env = run.env.clone();
        baseline.pages = run.pages.clone();
        run.session = Some(Arc::new(Self {
            sources,
            baseline,
            resolved: OnceLock::new(),
        }));
    }

    pub fn observed(run: &RunState) -> SessionResolution {
        run.session
            .as_ref()
            .and_then(|scope| scope.resolved.get())
            .cloned()
            .unwrap_or_else(|| resolve_source(None, run))
    }

    pub fn resolve(run: &RunState) -> SessionResolution {
        let Some(scope) = &run.session else {
            return resolve_source(None, run);
        };
        scope
            .resolved
            .get_or_init(|| {
                if let Some(source) = scope.sources.get(&None) {
                    return resolve_source(source.as_deref(), &scope.baseline);
                }
                let named: BTreeMap<_, _> = scope
                    .sources
                    .iter()
                    .map(|(name, source)| {
                        (
                            name.clone().expect("named scope"),
                            resolve_source(source.as_deref(), &scope.baseline),
                        )
                    })
                    .collect();
                SessionResolution {
                    source: None,
                    seed: None,
                    failed: named.values().any(|s| s.failed),
                    named,
                }
            })
            .clone()
    }
}

#[derive(Debug)]
pub(crate) struct ContextBudget {
    check: String,
    cap: usize,
    open: AtomicUsize,
}

impl ContextBudget {
    pub fn new(check: &str, cap: usize) -> Arc<Self> {
        Arc::new(Self {
            check: check.into(),
            cap,
            open: AtomicUsize::new(0),
        })
    }

    pub fn reserve(self: &Arc<Self>) -> Result<ContextPermit, EngineError> {
        self.open
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |open| {
                (open < self.cap).then(|| open + 1)
            })
            .map_err(|_| EngineError::SessionLimit {
                check: self.check.clone(),
                cap: self.cap,
            })?;
        Ok(ContextPermit(self.clone()))
    }
}

pub(crate) struct ContextPermit(Arc<ContextBudget>);
impl Drop for ContextPermit {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simultaneous_context_guard_fails_closed_and_releases_capacity() {
        let budget = ContextBudget::new("AC-1.1", 1);
        let permit = budget.reserve().unwrap();
        let error = budget.reserve().err().unwrap().to_string();
        assert!(
            error.contains("AC-1.1") && error.contains("max_sessions (1)"),
            "{error}"
        );
        drop(permit);
        assert!(budget.reserve().is_ok());
    }
}

#[cfg(test)]
mod seed_tests {
    use super::*;
    use crate::engine::context::json_to_value;

    #[test]
    fn lazy_seed_uses_enclosing_snapshot_and_stays_fixed_across_retries() {
        let baseline = serde_json::json!({"cookies": [], "origins": []});
        let mut run = RunState::new(BTreeMap::new());
        run.record_setup_output("login", "state", json_to_value(&baseline).unwrap());
        SessionScope::enter(
            &mut run,
            &Some(Some("$setup.login.outputs.state".into())),
            None,
        );
        assert!(run.session.as_ref().unwrap().resolved.get().is_none());
        run.record_setup_output("login", "state", crate::eval::Value::Null);
        let first = SessionScope::resolve(&run);
        assert_eq!(first.seed.as_ref().unwrap().state, baseline);
        run.setup_outputs.clear();
        assert_eq!(SessionScope::resolve(&run).digest(), first.digest());
        // Omission shares the immutable seed, while explicit null stops it.
        SessionScope::enter(&mut run, &None, None);
        assert_eq!(SessionScope::resolve(&run).digest(), first.digest());
        SessionScope::enter(&mut run, &Some(None), None);
        assert!(SessionScope::resolve(&run).seed.is_none());
    }
}
