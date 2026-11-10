//! Replay a recorded trace and assert the verdict reconstructs.
//!
//! The §11.2 reproducibility commitment is "identical environment +
//! frozen check spec → identical verdict on replay". This module is
//! the empirical verifier of that claim: re-aggregate the recorded
//! `assertion_evaluated` outcomes via `duhem-judge::aggregate_run`
//! and compare against the recorded `check_finished` /
//! `criterion_finished` / `run_finished` verdicts. Any mismatch is a
//! [`ReplayDivergence`].
//!
//! The judge owns the fold; this crate owns the on-disk format and
//! the divergence detection. Splitting the two is the OSS-judge
//! boundary in `docs/duhem-spec.md` §11.2 — the judge can be audited
//! and re-implemented without pulling in evidence machinery.

use std::collections::BTreeMap;

use duhem_judge::{
    AssertionOutcome, CheckOutcome, CheckVerdict, CriterionVerdict, RunVerdict, VerdictState,
    aggregate_check, aggregate_criterion, aggregate_run,
};
use thiserror::Error;

use crate::event::EventPayload;
use crate::reader::Trace;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReplayError {
    #[error("trace missing run_started event")]
    MissingRunStarted,
    #[error("trace missing run_finished event")]
    MissingRunFinished,
    #[error("{0}")]
    Divergence(#[from] ReplayDivergence),
    #[error("event ordering violated: {0}")]
    OutOfOrder(String),
}

/// A verdict the replay computed differs from what was recorded.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReplayDivergence {
    #[error("run body: recorded verdict {recorded}, recomputed {recomputed}")]
    Body {
        recorded: VerdictState,
        recomputed: VerdictState,
    },
    #[error("check {check_id}: recorded verdict {recorded}, recomputed {recomputed}")]
    Check {
        check_id: String,
        recorded: VerdictState,
        recomputed: VerdictState,
    },
    #[error("criterion {criterion_id}: recorded verdict {recorded}, recomputed {recomputed}")]
    Criterion {
        criterion_id: String,
        recorded: VerdictState,
        recomputed: VerdictState,
    },
    #[error("run: recorded verdict {recorded}, recomputed {recomputed}")]
    Run {
        recorded: VerdictState,
        recomputed: VerdictState,
    },
}

/// The aggregated outcome of a successful replay. Carries the same
/// `RunVerdict` shape the judge would produce live, so consumers can
/// render replay output identically to a fresh run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayedRun {
    pub run: RunVerdict,
}

/// Re-aggregate the recorded outcomes and assert reproducibility.
pub fn replay(trace: &Trace) -> Result<ReplayedRun, ReplayError> {
    let events = trace.events();

    // Per-check assertion outcomes (in trace order — first-cause-wins
    // for `Inconclusive` matches the judge's fold), recorded
    // verdicts, and check→criterion membership.
    let mut assertions_by_check: BTreeMap<String, Vec<AssertionOutcome>> = BTreeMap::new();
    let mut recorded_checks: BTreeMap<String, VerdictState> = BTreeMap::new();
    let mut recorded_criteria: BTreeMap<String, VerdictState> = BTreeMap::new();
    let mut check_to_criterion: BTreeMap<String, String> = BTreeMap::new();
    // Preserve criterion order from `criterion_finished` events so the
    // recomputed `RunVerdict.criteria` matches the recorded ordering.
    let mut criterion_order: Vec<String> = Vec::new();
    let mut recorded_run: Option<VerdictState> = None;
    let mut recorded_body = None;
    let mut required_cleanup = std::collections::BTreeSet::new();
    let mut cleanup_failed = false;
    let mut process_setup = crate::replay_process::ProcessSetup::default();
    let mut saw_run_started = false;

    // A step owner is stronger than the additive check_finished fallback,
    // regardless of event order. Preserve the existing hard error for two
    // conflicting step owners (#490).
    for evt in events {
        if let EventPayload::StepStarted {
            criterion_id,
            check_id,
            ..
        } = &evt.payload
        {
            match check_to_criterion.get(check_id) {
                Some(prev) if prev != criterion_id => {
                    return Err(ReplayError::OutOfOrder(format!(
                        "check {check_id} mapped to both criterion {prev} and {criterion_id}"
                    )));
                }
                _ => {
                    check_to_criterion.insert(check_id.clone(), criterion_id.clone());
                }
            }
        }
    }

    for evt in events {
        process_setup.observe(&evt.payload);
        match &evt.payload {
            EventPayload::RunStarted { .. } => {
                if saw_run_started {
                    return Err(ReplayError::OutOfOrder("more than one run_started".into()));
                }
                saw_run_started = true;
            }
            EventPayload::StepStarted { .. } => {}
            EventPayload::AssertionEvaluated {
                check_id,
                assertion_index,
                iteration,
                state,
                detail,
                ..
            } => {
                assertions_by_check
                    .entry(check_id.clone())
                    .or_default()
                    .push(AssertionOutcome {
                        iteration: *iteration,
                        assertion_index: *assertion_index as usize,
                        state: *state,
                        detail: detail.clone(),
                    });
            }
            EventPayload::CheckFinished {
                check_id,
                criterion_id,
                verdict,
                ..
            } => {
                if !check_to_criterion.contains_key(check_id)
                    && let Some(criterion_id) = criterion_id
                {
                    check_to_criterion.insert(check_id.clone(), criterion_id.clone());
                }
                recorded_checks.insert(check_id.clone(), *verdict);
                if !assertions_by_check.contains_key(check_id)
                    && let Some(state) = process_setup.failed_for(criterion_id.as_deref(), check_id)
                {
                    assertions_by_check.insert(
                        check_id.clone(),
                        vec![AssertionOutcome {
                            iteration: None,
                            assertion_index: 0,
                            state,
                            detail: Some("process setup precondition failed".into()),
                        }],
                    );
                }
            }
            EventPayload::CriterionFinished {
                criterion_id,
                verdict,
            } => {
                if !recorded_criteria.contains_key(criterion_id) {
                    criterion_order.push(criterion_id.clone());
                }
                recorded_criteria.insert(criterion_id.clone(), *verdict);
            }
            EventPayload::RunFinished {
                verdict,
                body_verdict,
            } => {
                recorded_run = *verdict;
                recorded_body = *body_verdict;
            }
            EventPayload::SetupStarted {
                phase: crate::StepPhase::Teardown,
                fixture_name,
                check_id,
                criterion_id,
                cleanup_policy,
                ..
            } => {
                let scope = (criterion_id.clone(), check_id.clone(), fixture_name.clone());
                if cleanup_policy.as_deref() == Some("required") {
                    required_cleanup.insert(scope);
                } else {
                    required_cleanup.remove(&scope);
                }
            }
            EventPayload::SetupStepFinished {
                phase: crate::StepPhase::Teardown,
                fixture_name,
                check_id,
                criterion_id,
                outcome,
                judgment,
                ..
            } if required_cleanup.contains(&(
                criterion_id.clone(),
                check_id.clone(),
                fixture_name.clone(),
            )) =>
            {
                cleanup_failed |= matches!(
                    outcome,
                    crate::StepOutcome::Error | crate::StepOutcome::Timeout
                ) || judgment.is_some_and(|state| state != VerdictState::Pass);
            }
            EventPayload::StepObservation { .. }
            | EventPayload::ProcessObserved { .. }
            | EventPayload::RunHeartbeat
            | EventPayload::ClaimScopeDeclared { .. }
            | EventPayload::RunAborted { .. }
            | EventPayload::StepFinished { .. }
            | EventPayload::SetupStarted { .. }
            | EventPayload::SetupStepStarted { .. }
            | EventPayload::SetupStepObservation { .. }
            | EventPayload::SetupStepFinished { .. }
            | EventPayload::SetupFinished { .. }
            | EventPayload::EnvUpStarted { .. }
            | EventPayload::EnvUpFinished { .. }
            | EventPayload::EnvReady { .. }
            | EventPayload::EnvDownStarted { .. }
            | EventPayload::EnvDownFinished { .. } => {
                // Setup / environment events don't produce
                // per-criterion verdicts — they're run-level boundary
                // markers. The judge's fold sees only criterion
                // verdicts, and `Engine::run` emits `RunFinished`
                // directly on setup-abort (#20) / env-abort (#50) so
                // replay still has the recorded run verdict to
                // compare against.
            }
        }
    }

    if !saw_run_started {
        return Err(ReplayError::MissingRunStarted);
    }
    let recorded_run = recorded_run.ok_or(ReplayError::MissingRunFinished)?;

    // Trace completeness: every check whose assertions were observed
    // must have a `check_finished`, and every criterion that owns an
    // observed check must have a `criterion_finished`. Without this
    // check, an incomplete trace could replay as "matching" simply
    // because the orphan assertions never got folded in.
    for check_id in assertions_by_check.keys() {
        if !recorded_checks.contains_key(check_id) {
            return Err(ReplayError::OutOfOrder(format!(
                "assertions recorded for check {check_id} but no check_finished"
            )));
        }
    }
    for criterion_id in check_to_criterion.values() {
        if !recorded_criteria.contains_key(criterion_id) {
            return Err(ReplayError::OutOfOrder(format!(
                "checks observed under criterion {criterion_id} but no criterion_finished"
            )));
        }
    }

    // Recompute check verdicts via the judge.
    let mut recomputed_check_verdicts: BTreeMap<String, CheckVerdict> = BTreeMap::new();
    for (check_id, recorded) in &recorded_checks {
        let outcome = CheckOutcome {
            check_id: check_id.clone(),
            assertions: assertions_by_check
                .get(check_id)
                .cloned()
                .unwrap_or_default(),
        };
        let recomputed = aggregate_check(&outcome);
        if recomputed.state != *recorded {
            return Err(ReplayDivergence::Check {
                check_id: check_id.clone(),
                recorded: *recorded,
                recomputed: recomputed.state,
            }
            .into());
        }
        recomputed_check_verdicts.insert(check_id.clone(), recomputed);
    }

    // Group recomputed check verdicts by criterion (preserving each
    // criterion's first-seen check ordering for evidence rendering).
    let mut checks_by_criterion: BTreeMap<String, Vec<CheckVerdict>> = BTreeMap::new();
    for (check_id, verdict) in &recomputed_check_verdicts {
        if let Some(criterion_id) = check_to_criterion.get(check_id) {
            checks_by_criterion
                .entry(criterion_id.clone())
                .or_default()
                .push(verdict.clone());
        }
    }

    // Recompute criterion verdicts via the judge, in recorded order.
    let mut recomputed_criteria: Vec<CriterionVerdict> = Vec::new();
    for criterion_id in &criterion_order {
        let recorded = recorded_criteria[criterion_id];
        let checks = checks_by_criterion.remove(criterion_id).unwrap_or_default();
        let state = aggregate_criterion(&checks);
        if state != recorded {
            return Err(ReplayDivergence::Criterion {
                criterion_id: criterion_id.clone(),
                recorded,
                recomputed: state,
            }
            .into());
        }
        recomputed_criteria.push(CriterionVerdict {
            criterion_id: criterion_id.clone(),
            state,
            checks,
        });
    }

    // Recompute the run verdict.
    let mut run = aggregate_run(recomputed_criteria);
    if let Some(recorded) = recorded_body
        && recorded != run.state
    {
        return Err(ReplayDivergence::Body {
            recorded,
            recomputed: run.state,
        }
        .into());
    }
    if cleanup_failed {
        run.state = VerdictState::Fail;
    }
    if run.state != recorded_run {
        return Err(ReplayDivergence::Run {
            recorded: recorded_run,
            recomputed: run.state,
        }
        .into());
    }

    Ok(ReplayedRun { run })
}
