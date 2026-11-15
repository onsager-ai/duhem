//! Distributed coverage is read-time policy; logical parents have no heartbeat.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use duhem_evidence::{RunOrigin, RunRecord, RunStatus, VerdictState};
use duhem_judge::InconclusiveCause;

use super::{
    EvidenceReader, ReaderError,
    tree::{descendant_run_ids, tree_verdict},
};
use crate::model::{MissingShard, ShardCoverage, ShardDetail};

impl EvidenceReader {
    pub(super) async fn coverage(
        &self,
        parent: &str,
        records: &[RunRecord],
        now: DateTime<Utc>,
    ) -> Result<Option<ShardCoverage>, ReaderError> {
        let mut arrived = BTreeMap::new();
        let mut expected = BTreeSet::new();
        let mut covered = BTreeSet::new();
        let mut last_arrival = None;
        for record in records.iter().filter(|record| {
            record.lineage.origin == Some(RunOrigin::Shard)
                && record.lineage.parent_run_id.as_deref() == Some(parent)
        }) {
            let events = self.store.run_events(&record.run_id).await?;
            let Some(scope) = duhem_evidence::claim_scope::claim_scope(&events) else {
                continue;
            };
            let (Some(shard), Some(partition)) = (
                scope.distributed.shard,
                scope.distributed.expected_partition,
            ) else {
                continue;
            };
            expected.extend(partition.units.iter().cloned());
            expected.extend(
                partition
                    .excluded
                    .iter()
                    .filter(|unit| unit.reason == "shard")
                    .map(|unit| unit.unit.clone()),
            );
            covered.extend(partition.units.iter().cloned());
            let arrival = record.received_at.unwrap_or(record.started_at);
            last_arrival =
                Some(last_arrival.map_or(arrival, |previous: DateTime<Utc>| previous.max(arrival)));
            let ids = descendant_run_ids(&record.run_id, records);
            let verdict = tree_verdict(self.store.as_ref(), records, &ids).await?;
            arrived.insert(
                shard.index,
                ShardDetail {
                    run_id: record.run_id.clone(),
                    index: shard.index,
                    total: shard.total,
                    source: shard.source,
                    unit_count: partition.units.len(),
                    verdict,
                    duration_ms: record.duration_ms,
                },
            );
        }
        let Some(first) = arrived.values().next() else {
            return Ok(None);
        };
        let total = first.total;
        let expected: Vec<_> = expected.into_iter().collect();
        let missing: Vec<_> = (1..=total)
            .filter(|index| !arrived.contains_key(index))
            .map(|index| MissingShard {
                index,
                // The same assignment every shard computed for itself.
                units: duhem_evidence::distributed::partition(&expected, index, total)
                    .into_iter()
                    .collect(),
            })
            .collect();
        let elapsed = now
            .signed_duration_since(last_arrival.expect("arrived shard has timestamp"))
            .to_std()
            .unwrap_or_default();
        let complete = missing.is_empty() && arrived.values().all(|shard| shard.verdict.is_some());
        let status = if complete || (!missing.is_empty() && elapsed >= self.shard_timeout) {
            RunStatus::Finished
        } else {
            RunStatus::Pending
        };
        Ok(Some(ShardCoverage {
            total,
            status,
            units: covered.into_iter().collect(),
            arrived: arrived.into_values().collect(),
            missing,
        }))
    }
}

pub(super) fn verdict(
    coverage: &ShardCoverage,
    complete: Option<VerdictState>,
) -> Option<VerdictState> {
    if coverage.missing.is_empty() {
        complete
    } else if coverage.status == RunStatus::Pending {
        None
    } else {
        Some(VerdictState::Inconclusive(
            InconclusiveCause::MissingObservation,
        ))
    }
}
