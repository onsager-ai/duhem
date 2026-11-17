//! Small run-detail projections kept out of the evidence reader fold.

use super::tree::{check_totals, descendant_run_ids, run_numbers, suite_children, tree_verdict};
use super::{ReaderError, build_run_detail, distributed, native};
use crate::model::{RunChildDetail, RunDetail};
use chrono::Utc;
use duhem_evidence::RunRecord;

use crate::model::RunSide;

pub(super) fn run_side(r: &RunRecord) -> RunSide {
    RunSide {
        run_id: r.run_id.clone(),
        started_at: Some(r.started_at),
        verdict: r.verdict,
    }
}

impl super::EvidenceReader {
    pub async fn run_detail(&self, run_id: &str) -> Result<Option<RunDetail>, ReaderError> {
        let Some(run) = self.load(run_id).await? else {
            return Ok(None);
        };
        let records = self.store.list_runs().await?;
        let tree_ids = descendant_run_ids(run_id, &records);
        let tree_wide = tree_ids.len() > 1;
        let children = if tree_wide {
            let mut children = Vec::new();
            for record in suite_children(run_id, &records) {
                let Some(child) = self.load(&record.run_id).await? else {
                    continue;
                };
                let child_ids = descendant_run_ids(&record.run_id, &records);
                let mut totals = check_totals(&build_run_detail(&child));
                for id in child_ids.iter().skip(1) {
                    if let Some(member) = self.load(id).await? {
                        let counts = check_totals(&build_run_detail(&member));
                        totals.pass += counts.pass;
                        totals.fail += counts.fail;
                        totals.inconclusive += counts.inconclusive;
                        totals.pending += counts.pending;
                        totals.total += counts.total;
                    }
                }
                children.push(RunChildDetail {
                    run_id: record.run_id.clone(),
                    verification: child.verification(),
                    verdict: tree_verdict(self.store.as_ref(), &records, &child_ids).await?,
                    status: child.record.status,
                    duration_ms: child.duration_ms(),
                    check_totals: totals,
                });
            }
            children
        } else {
            Vec::new()
        };
        let mut detail = build_run_detail(&run);
        native::populate(self, &run, &tree_ids, &mut detail).await?;
        detail.children = children;
        detail.run_number = run_numbers(&records)
            .get(run_id)
            .copied()
            .unwrap_or_default();
        if tree_wide {
            detail.verdict = tree_verdict(self.store.as_ref(), &records, &tree_ids).await?;
        }
        if let Some(coverage) = self.coverage(run_id, &records, Utc::now()).await? {
            detail.verdict = distributed::verdict(&coverage, detail.verdict);
            detail.status = coverage.status;
            detail.shards = Some(coverage);
        }
        let mut totals = check_totals(&detail);
        for child in &detail.children {
            totals.pass += child.check_totals.pass;
            totals.fail += child.check_totals.fail;
            totals.inconclusive += child.check_totals.inconclusive;
            totals.pending += child.check_totals.pending;
            totals.total += child.check_totals.total;
        }
        detail.check_totals = Some(totals);
        Ok(Some(detail))
    }
}
