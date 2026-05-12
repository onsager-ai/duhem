//! Run-level `setup:` execution.
//!
//! Per the spec on issue #20: setup runs once per run, before any
//! criterion, against its own browser context. Step outputs are
//! published into `RunState.setup_outputs` so checks can reference
//! them as `$setup.<id>.outputs.<name>`; browser state does *not*
//! cross the boundary — each check still opens its own browser per
//! issue #15.
//!
//! Failure policy is three-state-faithful (`docs/duhem-spec.md` §7.6):
//! `Outcome::Error` or `Outcome::Timeout` from any setup step aborts
//! setup, no criterion runs, and the run verdict is
//! `Inconclusive(EnvironmentError)` — "we couldn't observe the
//! workload in the state the Verification Definition claims to
//! verify". Distinct from `Fail`, which would mean we observed the
//! workload misbehaving.

use duhem_actions::{Outcome, RunBrowser};
use duhem_evidence::{EventPayload, EvidenceWriter};
use duhem_schema::Step;
use playwright::api::Page;
use tracing::debug;

use crate::engine::context::{RunState, json_to_value};
use crate::engine::registry::{ActionRegistry, Dispatch};
use crate::engine::runner::{EngineError, outcome_to_evidence, with_to_evidence_map};
use crate::engine::template::substitute_with;

/// Outcome of walking the run-level `setup:` block.
pub(crate) struct SetupResult {
    /// `true` when any step produced `Outcome::Error` or
    /// `Outcome::Timeout` and the rest of setup was skipped. Drives
    /// the engine's "skip criteria, emit Inconclusive" path.
    pub aborted: bool,
}

/// Execute every step in `setup` once, emitting `Setup*` evidence
/// events and recording any outputs onto `run.setup_outputs`.
/// Caller is responsible for skipping the call entirely when
/// `setup.is_empty()` so the wire shape stays byte-identical for
/// setup-free definitions.
pub(crate) async fn run_setup(
    writer: &mut EvidenceWriter,
    registry: &ActionRegistry,
    browser: Option<&RunBrowser>,
    run: &mut RunState,
    setup: &[Step],
) -> Result<SetupResult, EngineError> {
    writer.append(EventPayload::SetupStarted {
        step_count: setup.len() as u32,
    })?;

    // Decide up front whether any step in this block needs a real
    // page. Mirrors the per-check logic in `Engine::run_check` so
    // setup behaves the same way on an env-failure path.
    let needs_browser = setup.iter().any(|s| {
        registry
            .get(s.uses.as_str())
            .map(|d| d.requires_page())
            .unwrap_or(false)
    });
    let any_unknown = setup
        .iter()
        .any(|s| !registry.contains_key(s.uses.as_str()));
    let browser_missing = needs_browser && browser.is_none();
    let mut environment_failed = browser_missing || any_unknown;

    // Setup gets its own browser context, never shared with checks.
    let mut setup_browser = None;
    if !environment_failed
        && !setup.is_empty()
        && let Some(b) = browser
    {
        match b.open_check().await {
            Ok(cb) => setup_browser = Some(cb),
            Err(e) => {
                debug!(error = %e, "open_check for setup failed");
                environment_failed = true;
            }
        }
    }

    let mut aborted = false;
    for (idx, step) in setup.iter().enumerate() {
        // Setup steps see the run state (inputs, env, uuid, plus any
        // outputs already published by earlier setup steps in this
        // same block). The view is read-only against the run state —
        // we feed it through a `RunContext` to reuse the existing
        // template substitution.
        let ctx = crate::engine::context::RunContext::new(run);
        let mut resolved_with = step.with.clone();
        substitute_with(&mut resolved_with, &ctx);

        writer.append(EventPayload::SetupStepStarted {
            step_index: idx as u32,
            uses: step.uses.clone(),
            with: with_to_evidence_map(&resolved_with),
        })?;

        let outcome = if aborted || environment_failed {
            Outcome::Error
        } else {
            match registry.get(step.uses.as_str()) {
                None => Outcome::Error,
                Some(dispatcher) => {
                    let page_ref: Option<&Page> = setup_browser.as_ref().map(|cb| &cb.page);
                    invoke_and_record(
                        dispatcher.as_ref(),
                        page_ref,
                        idx,
                        &resolved_with,
                        step,
                        run,
                        writer,
                    )
                    .await?
                }
            }
        };

        writer.append(EventPayload::SetupStepFinished {
            step_index: idx as u32,
            outcome: outcome_to_evidence(&outcome),
        })?;

        if matches!(outcome, Outcome::Error | Outcome::Timeout) {
            aborted = true;
        }
    }

    if let Some(cb) = setup_browser {
        let _ = cb.close().await;
    }

    writer.append(EventPayload::SetupFinished { aborted })?;
    Ok(SetupResult { aborted })
}

/// Invoke one setup-step dispatcher, write a `SetupStepObservation`
/// for every output, and publish scalar outputs onto
/// `RunState.setup_outputs` so checks can reference them as
/// `$setup.<id>.outputs.<name>`.
async fn invoke_and_record(
    dispatcher: &dyn Dispatch,
    page: Option<&Page>,
    idx: usize,
    resolved_with: &serde_yml::Value,
    step: &Step,
    run: &mut RunState,
    writer: &mut EvidenceWriter,
) -> Result<Outcome, EngineError> {
    let result = dispatcher.invoke(page, idx, resolved_with).await;
    let outcome = match &result {
        Ok(r) => r.outcome.clone(),
        Err(_) => Outcome::Error,
    };
    if let Ok(r) = &result {
        for (name, value) in &r.outputs {
            if let Some(scalar) = json_to_value(value)
                && let Some(id) = step.id.as_deref()
            {
                run.record_setup_output(id, name, scalar);
            }
            // Setup observations get their own event variant so
            // readers can attribute the observation to the
            // run-level setup block, not a per-check step.
            append_setup_observation(writer, idx as u32, name.clone(), value.clone())?;
        }
    }
    Ok(outcome)
}

/// Mirror of `EvidenceWriter::append_observation` for setup. The
/// inline-vs-blob policy (`BLOB_INLINE_THRESHOLD_BYTES`) is shared;
/// only the event variant differs.
fn append_setup_observation(
    writer: &mut EvidenceWriter,
    step_index: u32,
    output_name: String,
    value: serde_json::Value,
) -> Result<(), EngineError> {
    use duhem_evidence::{BLOB_INLINE_THRESHOLD_BYTES, ObservationValue};
    let inline_bytes = serde_json::to_vec(&value).map_err(duhem_evidence::WriterError::from)?;
    let obs = if inline_bytes.len() > BLOB_INLINE_THRESHOLD_BYTES {
        let sha = writer.write_blob(&inline_bytes)?;
        ObservationValue::Blob { blob_sha256: sha.0 }
    } else {
        ObservationValue::Inline { value }
    };
    writer.append(EventPayload::SetupStepObservation {
        step_index,
        output_name,
        value: obs,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::registry::Dispatch;
    use async_trait::async_trait;
    use duhem_actions::{ActionError, ActionResult, Outcome};
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StubAction {
        uses: &'static str,
        outcome: Outcome,
        outputs: Vec<(&'static str, serde_json::Value)>,
        invocations: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Dispatch for StubAction {
        fn uses(&self) -> &'static str {
            self.uses
        }
        fn requires_page(&self) -> bool {
            false
        }
        async fn invoke(
            &self,
            _page: Option<&Page>,
            _step_index: usize,
            _with: &serde_yml::Value,
        ) -> Result<ActionResult, ActionError> {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            let mut r = match self.outcome {
                Outcome::Ok => ActionResult::ok(),
                Outcome::Error => ActionResult::error(),
                Outcome::Timeout => ActionResult::timeout(),
            };
            for (k, v) in &self.outputs {
                r = r.with_output(k, v.clone());
            }
            Ok(r)
        }
    }

    fn make_writer() -> (EvidenceWriter, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path().join("run");
        let w = EvidenceWriter::new(&run_dir, "x.yml").unwrap();
        (w, tmp)
    }

    fn step(id: Option<&str>, uses: &str) -> Step {
        Step {
            id: id.map(String::from),
            uses: uses.to_string(),
            with: serde_yml::Value::Null,
            outputs: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn setup_publishes_outputs_into_run_state() {
        let (mut w, _tmp) = make_writer();
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/seed",
            Box::new(StubAction {
                uses: "fake/seed",
                outcome: Outcome::Ok,
                outputs: vec![("token", serde_json::json!("abc"))],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        let setup = vec![step(Some("warm"), "fake/seed")];
        let r = run_setup(&mut w, &registry, None, &mut run, &setup)
            .await
            .unwrap();
        assert!(!r.aborted);
        assert_eq!(
            run.setup_outputs.get(&("warm".into(), "token".into())),
            Some(&crate::eval::Value::Str("abc".into())),
        );
    }

    #[tokio::test]
    async fn setup_aborts_on_first_error() {
        let (mut w, _tmp) = make_writer();
        let mut registry: ActionRegistry = BTreeMap::new();
        registry.insert(
            "fake/boom",
            Box::new(StubAction {
                uses: "fake/boom",
                outcome: Outcome::Error,
                outputs: vec![],
                invocations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let after = Arc::new(AtomicUsize::new(0));
        registry.insert(
            "fake/tracker",
            Box::new(StubAction {
                uses: "fake/tracker",
                outcome: Outcome::Ok,
                outputs: vec![],
                invocations: after.clone(),
            }),
        );
        let mut run = RunState::new(BTreeMap::new());
        let setup = vec![step(None, "fake/boom"), step(None, "fake/tracker")];
        let r = run_setup(&mut w, &registry, None, &mut run, &setup)
            .await
            .unwrap();
        assert!(r.aborted, "setup should abort after Error");
        assert_eq!(
            after.load(Ordering::SeqCst),
            0,
            "step after Error must not invoke"
        );
    }
}
