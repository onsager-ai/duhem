//! Replay a recorded trace and assert the verdict reconstructs.
//!
//! The §11.2 reproducibility commitment is "identical environment +
//! frozen check spec → identical verdict on replay". This module is
//! the empirical verifier of that claim: re-aggregate the recorded
//! `assertion_evaluated` outcomes and compare against the recorded
//! `check_finished` / `criterion_finished` / `run_finished` verdicts.
//! Any mismatch is a [`ReplayDivergence`].
//!
//! The aggregation rule used here is the minimal one needed to land
//! evidence v1 in parallel with the judge spec
//! (`spec(judge): three-state verdict aggregation rules`):
//!
//! - A check's verdict is `fail` if any of its assertions failed,
//!   `inconclusive` if any are inconclusive (and none failed),
//!   otherwise `pass`.
//! - A criterion's verdict is the same fold over its checks.
//! - A run's verdict is the same fold over its criteria.
//!
//! When the judge crate lands its canonical `aggregate_run`, replay
//! delegates to it and this helper goes away. Until then, this local
//! version is the contract — it must be byte-for-byte identical to
//! what the runtime computed during the original run.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::event::{AssertionState, EventPayload, Verdict};
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
    #[error("check {check_id}: recorded verdict {recorded:?}, recomputed {recomputed:?}")]
    Check {
        check_id: String,
        recorded: Verdict,
        recomputed: Verdict,
    },
    #[error("criterion {criterion_id}: recorded verdict {recorded:?}, recomputed {recomputed:?}")]
    Criterion {
        criterion_id: String,
        recorded: Verdict,
        recomputed: Verdict,
    },
    #[error("run: recorded verdict {recorded:?}, recomputed {recomputed:?}")]
    Run {
        recorded: Verdict,
        recomputed: Verdict,
    },
}

/// The aggregated outcome of a replay. Only present when the recorded
/// verdict matches the recomputed verdict end to end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunVerdict {
    pub run: Verdict,
    pub criteria: BTreeMap<String, Verdict>,
    pub checks: BTreeMap<String, Verdict>,
}

/// Fold three assertion states into a single verdict using the
/// fail-dominates / inconclusive-second precedence described in
/// `docs/duhem-spec.md` §7.6.
fn fold_assertions(states: &[AssertionState]) -> Verdict {
    let mut has_inconclusive = false;
    for s in states {
        match s {
            AssertionState::Fail => return Verdict::Fail,
            AssertionState::Inconclusive => has_inconclusive = true,
            AssertionState::Pass => {}
        }
    }
    if has_inconclusive {
        Verdict::Inconclusive
    } else {
        Verdict::Pass
    }
}

fn fold_verdicts(verdicts: &[Verdict]) -> Verdict {
    let mut has_inconclusive = false;
    for v in verdicts {
        match v {
            Verdict::Fail => return Verdict::Fail,
            Verdict::Inconclusive => has_inconclusive = true,
            Verdict::Pass => {}
        }
    }
    if has_inconclusive {
        Verdict::Inconclusive
    } else {
        Verdict::Pass
    }
}

/// Re-aggregate the recorded outcomes and assert reproducibility.
pub fn replay(trace: &Trace) -> Result<RunVerdict, ReplayError> {
    let events = trace.events();

    // Collect assertions per check, recorded check verdicts, criterion
    // membership (check → criterion), and the recorded run verdict.
    let mut assertions_by_check: BTreeMap<String, Vec<AssertionState>> = BTreeMap::new();
    let mut recorded_checks: BTreeMap<String, Verdict> = BTreeMap::new();
    let mut recorded_criteria: BTreeMap<String, Verdict> = BTreeMap::new();
    let mut check_to_criterion: BTreeMap<String, String> = BTreeMap::new();
    let mut recorded_run: Option<Verdict> = None;
    let mut saw_run_started = false;

    for evt in events {
        match &evt.payload {
            EventPayload::RunStarted { .. } => {
                if saw_run_started {
                    return Err(ReplayError::OutOfOrder("more than one run_started".into()));
                }
                saw_run_started = true;
            }
            EventPayload::StepStarted {
                criterion_id,
                check_id,
                ..
            } => {
                // A check belongs to exactly one criterion. A trace
                // that maps the same check_id under two different
                // criteria is malformed — refuse it rather than
                // silently letting "last write wins" mis-attribute
                // recomputed verdicts.
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
            EventPayload::AssertionEvaluated {
                check_id, state, ..
            } => {
                assertions_by_check
                    .entry(check_id.clone())
                    .or_default()
                    .push(*state);
            }
            EventPayload::CheckFinished { check_id, verdict } => {
                recorded_checks.insert(check_id.clone(), *verdict);
            }
            EventPayload::CriterionFinished {
                criterion_id,
                verdict,
            } => {
                recorded_criteria.insert(criterion_id.clone(), *verdict);
            }
            EventPayload::RunFinished { verdict } => {
                recorded_run = Some(*verdict);
            }
            EventPayload::StepObservation { .. } | EventPayload::StepFinished { .. } => {}
        }
    }

    if !saw_run_started {
        return Err(ReplayError::MissingRunStarted);
    }
    let recorded_run = recorded_run.ok_or(ReplayError::MissingRunFinished)?;

    // Trace completeness: every check whose assertions were observed
    // must have a `check_finished`, and every criterion that owns an
    // observed check must have a `criterion_finished`. Without this,
    // an incomplete trace could replay as "matching" simply because
    // the orphan assertions never got folded in.
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

    // Recompute check verdicts; fall back to inconclusive if the check
    // finished with no recorded assertions (e.g. step error before any
    // assertion fired).
    let mut recomputed_checks: BTreeMap<String, Verdict> = BTreeMap::new();
    for (check_id, recorded) in &recorded_checks {
        let recomputed = match assertions_by_check.get(check_id) {
            Some(states) if !states.is_empty() => fold_assertions(states),
            _ => Verdict::Inconclusive,
        };
        if recomputed != *recorded {
            return Err(ReplayDivergence::Check {
                check_id: check_id.clone(),
                recorded: *recorded,
                recomputed,
            }
            .into());
        }
        recomputed_checks.insert(check_id.clone(), recomputed);
    }

    // Recompute criterion verdicts.
    let mut checks_by_criterion: BTreeMap<String, Vec<Verdict>> = BTreeMap::new();
    for (check_id, verdict) in &recomputed_checks {
        if let Some(criterion_id) = check_to_criterion.get(check_id) {
            checks_by_criterion
                .entry(criterion_id.clone())
                .or_default()
                .push(*verdict);
        }
    }
    let mut recomputed_criteria: BTreeMap<String, Verdict> = BTreeMap::new();
    for (criterion_id, recorded) in &recorded_criteria {
        let verdicts = checks_by_criterion
            .get(criterion_id)
            .cloned()
            .unwrap_or_default();
        let recomputed = if verdicts.is_empty() {
            Verdict::Inconclusive
        } else {
            fold_verdicts(&verdicts)
        };
        if recomputed != *recorded {
            return Err(ReplayDivergence::Criterion {
                criterion_id: criterion_id.clone(),
                recorded: *recorded,
                recomputed,
            }
            .into());
        }
        recomputed_criteria.insert(criterion_id.clone(), recomputed);
    }

    // Recompute run verdict.
    let criterion_verdicts: Vec<Verdict> = recomputed_criteria.values().copied().collect();
    let recomputed_run = if criterion_verdicts.is_empty() {
        Verdict::Inconclusive
    } else {
        fold_verdicts(&criterion_verdicts)
    };
    if recomputed_run != recorded_run {
        return Err(ReplayDivergence::Run {
            recorded: recorded_run,
            recomputed: recomputed_run,
        }
        .into());
    }

    Ok(RunVerdict {
        run: recomputed_run,
        criteria: recomputed_criteria,
        checks: recomputed_checks,
    })
}
