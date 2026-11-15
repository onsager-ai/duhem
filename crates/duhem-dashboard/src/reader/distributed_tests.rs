use super::EvidenceReader;
use chrono::{Duration, Utc};
use duhem_evidence::{EvidenceWriter, RunLineage, RunOrigin, RunStatus, SqliteStore, Store};
use duhem_summary::{ClaimScope, DistributedRun, ExpectedPartition, ShardIdentity};
use std::sync::Arc;

#[tokio::test]
async fn logical_parent_does_not_expire_with_heartbeat_and_uses_last_receipt() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(tmp.path().join("runs.db")).await.unwrap());
    store
        .begin_run(&duhem_evidence::RunMeta {
            run_id: "parent".into(),
            verification: "leaf.yml".into(),
            schema_version: "v1".into(),
            inputs: Default::default(),
            started_at: Utc::now() - Duration::hours(2),
            lineage: Default::default(),
            scope: Default::default(),
        })
        .await
        .unwrap();
    let mut shard = EvidenceWriter::begin_scoped_with_secrets_and_lineage(
        store.clone(),
        "shard",
        "leaf.yml",
        Default::default(),
        Default::default(),
        RunLineage {
            parent_run_id: Some("parent".into()),
            origin: Some(RunOrigin::Shard),
        },
        duhem_evidence::SecretRegistry::new(),
    )
    .await
    .unwrap();
    let scope = ClaimScope {
        distributed: DistributedRun {
            shard: Some(ShardIdentity {
                index: 1,
                total: 2,
                source: "flag".into(),
            }),
            expected_partition: Some(ExpectedPartition {
                units: vec!["a".into()],
                total_units: 2,
                excluded: vec![duhem_summary::ExcludedUnit {
                    unit: "b".into(),
                    reason: "shard".into(),
                }],
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    shard
        .append(duhem_evidence::EventPayload::ClaimScopeDeclared {
            scope: Box::new(scope),
        })
        .await
        .unwrap();
    shard.finish().await.unwrap();
    let mut records = store.list_runs().await.unwrap();
    let old = Utc::now() - Duration::hours(2);
    for record in &mut records {
        record.started_at = old;
        record.last_heartbeat_at = old;
        if record.run_id == "shard" {
            record.received_at = Some(Utc::now());
        }
    }
    assert_eq!(
        store.get_run("parent").await.unwrap().unwrap().status,
        RunStatus::Pending
    );
    let reader = EvidenceReader::new(store);
    let coverage = reader
        .coverage("parent", &records, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(coverage.status, RunStatus::Pending);
    let coverage = reader
        .coverage("parent", &records, Utc::now() + Duration::hours(1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(coverage.status, RunStatus::Finished);
    assert_eq!(coverage.missing[0].units, ["b"]);
}
