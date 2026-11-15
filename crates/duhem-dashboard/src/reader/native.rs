//! Native evidence projections; masking is performed by the evidence writer.
use duhem_evidence::{EventPayload, VerdictState};
use duhem_summary::{CheckTotals, NativeRunSummary};

use super::{RunEvidence, build_run_detail};
use crate::model::{ArtifactRef, FailingNative, NativeRunDetail, RunDetail};

fn bounded(text: &str) -> String {
    let mut chars = text.chars();
    let mut output: String = chars.by_ref().take(2048).collect();
    if chars.next().is_some() {
        output.push_str("… [truncated]");
    }
    output
}

pub(super) fn summaries(run: &RunEvidence) -> Vec<NativeRunDetail> {
    run.events
        .iter()
        .filter_map(|event| {
            let EventPayload::NativeRunFinished { result } = &event.payload else {
                return None;
            };
            let mut summary = result.clone();
            summary.detail = summary.detail.as_deref().map(bounded);
            let evidence = [
                ("stdout", &summary.stdout_blob_sha256),
                ("stderr", &summary.stderr_blob_sha256),
                ("report", &summary.report_blob_sha256),
                ("results", &summary.results_blob_sha256),
            ]
            .into_iter()
            .filter_map(|(kind, sha)| {
                sha.as_ref().map(|sha| ArtifactRef {
                    id: sha.clone(),
                    kind: kind.into(),
                    url: format!("/api/runs/{}/artifact/{sha}", run.record.run_id),
                    session: None,
                    mask_counts: Default::default(),
                })
            })
            .collect();
            Some(NativeRunDetail {
                run_id: run.record.run_id.clone(),
                summary,
                evidence,
            })
        })
        .collect()
}

pub(super) fn failures(run: &RunEvidence) -> Vec<FailingNative> {
    summaries(run)
        .into_iter()
        .filter(|entry| entry.summary.outcome != VerdictState::Pass)
        .map(|entry| FailingNative {
            run_id: entry.run_id,
            native_id: entry.summary.id,
            layer: entry.summary.layer,
            outcome: entry.summary.outcome,
            required: entry.summary.required,
            detail: entry.summary.detail,
            failing_tests: entry.summary.failing_tests,
            evidence: entry.evidence,
        })
        .collect()
}

/// Count recorded checks once per member; sum their execution intervals, not
/// the suite parent's wall time (which includes children and native commands).
pub(super) fn check_stats(run: &RunEvidence) -> (CheckTotals, u64) {
    let detail = build_run_detail(run);
    let mut totals = CheckTotals::default();
    for check in detail
        .criteria
        .iter()
        .flat_map(|criterion| &criterion.checks)
    {
        if check.verdict.is_some() {
            totals.total += 1;
        }
        match check.verdict {
            Some(VerdictState::Pass) => totals.passed += 1,
            Some(VerdictState::Fail) => totals.failed += 1,
            Some(VerdictState::Inconclusive(_)) => totals.inconclusive += 1,
            None => {}
        }
    }
    let mut start = None;
    let mut duration = 0;
    for event in &run.events {
        match &event.payload {
            EventPayload::StepStarted { .. } => start = Some(event.ts),
            EventPayload::StepFinished { .. } => {
                if let Some(started) = start.take() {
                    duration += (event.ts - started).num_milliseconds().max(0) as u64;
                }
            }
            _ => {}
        }
    }
    (totals, duration)
}

pub(super) async fn populate(
    reader: &super::EvidenceReader,
    run: &RunEvidence,
    tree_ids: &[String],
    detail: &mut RunDetail,
) -> Result<(), super::ReaderError> {
    let (mut checks, mut duration) = check_stats(run);
    detail.native = summaries(run);
    for id in tree_ids.iter().filter(|id| **id != run.record.run_id) {
        if let Some(member) = reader.load(id).await? {
            detail.native.extend(summaries(&member));
            let (totals, elapsed) = check_stats(&member);
            checks.total += totals.total;
            checks.passed += totals.passed;
            checks.failed += totals.failed;
            checks.inconclusive += totals.inconclusive;
            duration += elapsed;
        }
    }
    if !detail.native.is_empty() || tree_ids.len() > 1 {
        let summaries: Vec<NativeRunSummary> = detail
            .native
            .iter()
            .map(|entry| entry.summary.clone())
            .collect();
        detail.layers = duhem_summary::layer_totals(&summaries, checks, duration);
    }
    Ok(())
}
