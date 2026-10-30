//! Authored cleanup and required resource finalizers after interruption.
use super::*;

impl Engine {
    pub(super) async fn drain_leaf_teardown(
        &self,
        writer: &mut EvidenceWriter,
        run: &mut RunState,
        def: &VerificationDefinition,
        run_id: &str,
    ) -> Vec<CleanupFailure> {
        run.begin_process_cleanup(&run.process_owner.clone());
        let mut cleanup = if self.keep_env || def.teardown.is_empty() {
            Vec::new()
        } else {
            match crate::engine::setup::run_teardown(
                writer,
                &self.registry,
                self.browser.as_ref(),
                run,
                &def.teardown,
                &self.child_process_env(run_id),
            )
            .await
            {
                Ok(failures) => failures,
                Err(error) => vec![CleanupFailure {
                    required: false,
                    step: "teardown evidence".to_string(),
                    outcome: duhem_evidence::StepOutcome::Error,
                    detail: Some(writer.mask_text(&error.to_string())),
                }],
            }
        };
        cleanup.extend(run.processes.lock().await.finalize(writer, None).await);
        cleanup
    }

    pub(crate) async fn drain_interrupted_processes(
        &self,
        writer: &mut EvidenceWriter,
        run: &mut RunState,
    ) -> Vec<CleanupFailure> {
        let mut failures = Vec::new();
        // Preserve the established cancellation contract for runs with no
        // process actions. This stack exists for the new owned resources.
        if !run.processes.lock().await.has_entries() {
            return failures;
        }
        while let Some(hook) = run.process_cleanup.pop() {
            run.process_owner = hook.owner.clone();
            run.session = hook.session;
            run.context_budget = hook.budget;
            let env = self.child_process_env(writer.run_id());
            let result =
                if let (Some(fixture), Some(check)) = (&hook.owner.fixture, &hook.owner.check) {
                    crate::engine::setup::run_fixture_down(
                        writer,
                        &self.registry,
                        self.browser.as_ref(),
                        run,
                        fixture,
                        check,
                        &hook.steps,
                        hook.policy,
                        &env,
                    )
                    .await
                } else if let (Some(criterion), Some(check)) =
                    (&hook.owner.criterion, &hook.owner.check)
                {
                    crate::engine::setup::run_check_teardown(
                        writer,
                        &self.registry,
                        self.browser.as_ref(),
                        run,
                        criterion,
                        check,
                        &hook.steps,
                        &env,
                    )
                    .await
                } else if let Some(criterion) = &hook.owner.criterion {
                    crate::engine::setup::run_criterion_teardown(
                        writer,
                        &self.registry,
                        self.browser.as_ref(),
                        run,
                        criterion,
                        &hook.steps,
                        &env,
                    )
                    .await
                } else if !self.keep_env {
                    crate::engine::setup::run_teardown(
                        writer,
                        &self.registry,
                        self.browser.as_ref(),
                        run,
                        &hook.steps,
                        &env,
                    )
                    .await
                } else {
                    Ok(Vec::new())
                };
            if let Ok(mut cleanup) = result {
                failures.append(&mut cleanup);
            }
            failures.extend(
                run.processes
                    .lock()
                    .await
                    .finalize(writer, Some(&hook.owner))
                    .await,
            );
        }
        // A signal may interrupt an already-running teardown (its hook has
        // been removed), so drain any resources that block was still owning.
        failures.extend(run.processes.lock().await.finalize(writer, None).await);
        failures
    }
}
