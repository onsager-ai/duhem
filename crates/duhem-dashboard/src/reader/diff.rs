//! `#211` run-to-run diff: fold each run into a comparable projection
//! and diff them criterion-by-criterion. Split out of `reader.rs` to keep
//! that file inside the prod-token budget (same reason as `mime`).

use duhem_evidence::{EventPayload, ObservationValue, VerdictState};

use super::RunEvidence;
use crate::model::{ArtifactRef, AssertionDiff, CheckDiff, CriterionDiff};

/// A run folded into the projection the diff compares over: ordered
/// criteria + checks with verdicts, each check's assertions and blob
/// artifacts. One pass over the event stream (same folds as
/// `build_run_detail` / `build_check_detail`, collected together).
pub(super) struct RunProjection {
    pub(super) criteria: Vec<(String, Option<VerdictState>)>,
    pub(super) checks: Vec<CheckProjection>,
}

pub(super) struct CheckProjection {
    pub(super) criterion_id: String,
    pub(super) check_id: String,
    pub(super) verdict: Option<VerdictState>,
    /// `(assertion_index, state, detail)` in recorded order.
    pub(super) assertions: Vec<(u32, VerdictState, Option<String>)>,
    pub(super) artifacts: Vec<ArtifactRef>,
}

pub(super) fn project_run(run: &RunEvidence) -> RunProjection {
    let mut criteria: Vec<(String, Option<VerdictState>)> = Vec::new();
    let mut checks: Vec<CheckProjection> = Vec::new();
    // Positional attribution for blob observations: they belong to the
    // check whose `step_started` most recently opened (same rule as
    // `build_check_detail`), so trailing `capture/*` observations land
    // on their check.
    let mut current = None;

    for evt in &run.events {
        match &evt.payload {
            EventPayload::StepStarted {
                criterion_id,
                check_id,
                ..
            } => {
                if !criteria.iter().any(|(c, _)| c == criterion_id) {
                    criteria.push((criterion_id.clone(), None));
                }
                let pos = match checks.iter().position(|c| &c.check_id == check_id) {
                    Some(p) => p,
                    None => {
                        checks.push(CheckProjection {
                            criterion_id: criterion_id.clone(),
                            check_id: check_id.clone(),
                            verdict: None,
                            assertions: Vec::new(),
                            artifacts: Vec::new(),
                        });
                        checks.len() - 1
                    }
                };
                current = Some(pos);
            }
            EventPayload::StepObservation {
                output_name,
                value:
                    ObservationValue::Blob {
                        blob_sha256,
                        mask_counts,
                    },
                ..
            } => {
                if let Some(pos) = current {
                    checks[pos].artifacts.push(ArtifactRef {
                        id: blob_sha256.clone(),
                        kind: output_name.clone(),
                        url: format!("/api/runs/{}/artifact/{}", run.record.run_id, blob_sha256),
                        mask_counts: mask_counts.clone(),
                    });
                }
            }
            EventPayload::AssertionEvaluated {
                check_id,
                assertion_index,
                state,
                detail,
                ..
            } => {
                if let Some(pos) = checks.iter().position(|c| &c.check_id == check_id) {
                    checks[pos]
                        .assertions
                        .push((*assertion_index, *state, detail.clone()));
                }
            }
            EventPayload::CheckFinished {
                check_id, verdict, ..
            } => {
                if let Some(pos) = checks.iter().position(|c| &c.check_id == check_id) {
                    checks[pos].verdict = Some(*verdict);
                }
            }
            EventPayload::CriterionFinished {
                criterion_id,
                verdict,
            } => {
                if let Some(entry) = criteria.iter_mut().find(|(c, _)| c == criterion_id) {
                    entry.1 = Some(*verdict);
                }
            }
            _ => {}
        }
    }
    RunProjection { criteria, checks }
}

/// Ordered union of criterion ids (current first, then baseline-only)
/// with per-criterion / per-check / per-assertion transitions. When
/// there is no baseline, `changed` is always `false` — the view shows
/// the "no passing baseline" state rather than painting everything as
/// changed.
pub(super) fn diff_criteria(
    cur: &RunProjection,
    base: Option<&RunProjection>,
) -> Vec<CriterionDiff> {
    let has_base = base.is_some();
    let mut ids: Vec<String> = cur.criteria.iter().map(|(c, _)| c.clone()).collect();
    if let Some(b) = base {
        for (c, _) in &b.criteria {
            if !ids.contains(c) {
                ids.push(c.clone());
            }
        }
    }
    ids.into_iter()
        .map(|cid| {
            let cur_v = cur
                .criteria
                .iter()
                .find(|(c, _)| *c == cid)
                .and_then(|(_, v)| *v);
            let base_v = base
                .and_then(|b| b.criteria.iter().find(|(c, _)| *c == cid))
                .and_then(|(_, v)| *v);
            CriterionDiff {
                id: cid.clone(),
                baseline_verdict: base_v,
                current_verdict: cur_v,
                changed: has_base && base_v != cur_v,
                checks: diff_checks(&cid, cur, base),
            }
        })
        .collect()
}

pub(super) fn diff_checks(
    criterion_id: &str,
    cur: &RunProjection,
    base: Option<&RunProjection>,
) -> Vec<CheckDiff> {
    let has_base = base.is_some();
    let mut ids: Vec<String> = cur
        .checks
        .iter()
        .filter(|c| c.criterion_id == criterion_id)
        .map(|c| c.check_id.clone())
        .collect();
    if let Some(b) = base {
        for c in b.checks.iter().filter(|c| c.criterion_id == criterion_id) {
            if !ids.contains(&c.check_id) {
                ids.push(c.check_id.clone());
            }
        }
    }
    ids.into_iter()
        .map(|cid| {
            let cur_c = cur.checks.iter().find(|c| c.check_id == cid);
            let base_c = base.and_then(|b| b.checks.iter().find(|c| c.check_id == cid));
            let cur_v = cur_c.and_then(|c| c.verdict);
            let base_v = base_c.and_then(|c| c.verdict);
            CheckDiff {
                id: cid,
                baseline_verdict: base_v,
                current_verdict: cur_v,
                changed: has_base && base_v != cur_v,
                assertions: diff_assertions(cur_c, base_c, has_base),
                baseline_artifacts: base_c.map(|c| c.artifacts.clone()).unwrap_or_default(),
                current_artifacts: cur_c.map(|c| c.artifacts.clone()).unwrap_or_default(),
            }
        })
        .collect()
}

pub(super) fn diff_assertions(
    cur: Option<&CheckProjection>,
    base: Option<&CheckProjection>,
    has_base: bool,
) -> Vec<AssertionDiff> {
    let mut idxs: Vec<u32> = cur
        .map(|c| c.assertions.iter().map(|(i, _, _)| *i).collect())
        .unwrap_or_default();
    if let Some(b) = base {
        for (i, _, _) in &b.assertions {
            if !idxs.contains(i) {
                idxs.push(*i);
            }
        }
    }
    idxs.sort_unstable();
    idxs.into_iter()
        .map(|i| {
            let c = cur.and_then(|c| c.assertions.iter().find(|(j, _, _)| *j == i));
            let b = base.and_then(|b| b.assertions.iter().find(|(j, _, _)| *j == i));
            let cur_state = c.map(|(_, s, _)| *s);
            let base_state = b.map(|(_, s, _)| *s);
            let cur_detail = c.and_then(|(_, _, d)| d.clone());
            let base_detail = b.and_then(|(_, _, d)| d.clone());
            let changed = has_base && (base_state != cur_state || base_detail != cur_detail);
            AssertionDiff {
                assertion_index: i,
                baseline_state: base_state,
                current_state: cur_state,
                baseline_detail: base_detail,
                current_detail: cur_detail,
                changed,
            }
        })
        .collect()
}
