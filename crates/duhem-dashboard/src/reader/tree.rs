//! The manifest run tree: suite membership, leaf verdict aggregation and
//! per-child check totals, shared by run detail and the failure envelope.

use std::collections::BTreeMap;

use duhem_evidence::{RunOrigin, RunRecord, VerdictState};

use crate::model::{RunCheckTotals, RunDetail};

/// The run tree under `root_id`: the root plus its manifest (`suite`)
/// children, recursively. Nested `invocation` runs started from inside a
/// check are evidence of that check, not parts of the whole run (Duhem's
/// own self-verification deliberately runs failing controls that way).
pub(super) fn descendant_run_ids(root_id: &str, records: &[RunRecord]) -> Vec<String> {
    let mut children: BTreeMap<&str, Vec<&RunRecord>> = BTreeMap::new();
    for record in records {
        if record.lineage.origin != Some(RunOrigin::Suite) {
            continue;
        }
        if let Some(parent) = record.lineage.parent_run_id.as_deref() {
            children.entry(parent).or_default().push(record);
        }
    }
    fn visit(run_id: &str, children: &BTreeMap<&str, Vec<&RunRecord>>, output: &mut Vec<String>) {
        output.push(run_id.to_string());
        let mut descendants = children.get(run_id).cloned().unwrap_or_default();
        descendants.sort_by_key(|record| (record.started_at, record.run_id.as_str()));
        for child in descendants {
            visit(&child.run_id, children, output);
        }
    }
    if records.iter().any(|record| record.run_id == root_id) {
        let mut output = Vec::new();
        visit(root_id, &children, &mut output);
        output
    } else {
        vec![root_id.to_string()]
    }
}

/// Aggregate the recorded verdicts of suite-tree leaves. Any unjudged leaf
/// leaves the whole tree unjudged, matching the failure-envelope contract.
pub(super) async fn tree_verdict(
    store: &dyn duhem_evidence::Store,
    records: &[RunRecord],
    tree_ids: &[String],
) -> Result<Option<VerdictState>, duhem_evidence::StoreError> {
    use duhem_judge::{RunVerdict, aggregate_run_set};
    let by_id: BTreeMap<&str, &RunRecord> = records
        .iter()
        .map(|record| (record.run_id.as_str(), record))
        .collect();
    let mut leaves: Vec<&RunRecord> = tree_ids
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .filter(|record| {
            !records.iter().any(|candidate| {
                candidate.lineage.parent_run_id.as_deref() == Some(record.run_id.as_str())
                    && tree_ids.iter().any(|tree_id| tree_id == &candidate.run_id)
            })
        })
        .collect();
    leaves.sort_by_key(|record| (record.started_at, record.run_id.as_str()));
    let leaf_verdicts: Option<Vec<RunVerdict>> = leaves
        .iter()
        .map(|record| {
            record.verdict.map(|state| RunVerdict {
                state,
                criteria: Vec::new(),
            })
        })
        .collect();
    let Some(verdicts) = leaf_verdicts else {
        return Ok(None);
    };
    let mut verdict = aggregate_run_set(verdicts).state;
    // Leaf RunFinished verdicts already include their native gate. Suite
    // parents have no judged body, so fold their recorded native outcomes here.
    for id in tree_ids {
        if leaves.iter().any(|leaf| &leaf.run_id == id) {
            continue;
        }
        let events = store.run_events(id).await?;
        verdict = duhem_judge::native_gate(
            verdict,
            events.iter().filter_map(|event| match &event.payload {
                duhem_evidence::EventPayload::NativeRunFinished { result } => {
                    Some((result.required, result.outcome))
                }
                _ => None,
            }),
        );
    }
    Ok(Some(verdict))
}

/// Direct `suite` children of `run_id`, in execution order.
pub(super) fn suite_children<'a>(run_id: &str, records: &'a [RunRecord]) -> Vec<&'a RunRecord> {
    let mut children: Vec<&RunRecord> = records
        .iter()
        .filter(|record| {
            record.lineage.parent_run_id.as_deref() == Some(run_id)
                && record.lineage.origin == Some(RunOrigin::Suite)
        })
        .collect();
    children.sort_by_key(|record| (record.started_at, record.run_id.as_str()));
    children
}

/// Check verdict tallies for one run's detail.
pub(super) fn check_totals(detail: &RunDetail) -> RunCheckTotals {
    let mut totals = RunCheckTotals {
        pass: 0,
        fail: 0,
        inconclusive: 0,
        pending: 0,
        total: 0,
    };
    for check in detail
        .criteria
        .iter()
        .flat_map(|criterion| &criterion.checks)
    {
        totals.total += 1;
        match check.verdict {
            Some(VerdictState::Pass) => totals.pass += 1,
            Some(VerdictState::Fail) => totals.fail += 1,
            Some(VerdictState::Inconclusive(_)) => totals.inconclusive += 1,
            None => totals.pending += 1,
        }
    }
    totals
}
