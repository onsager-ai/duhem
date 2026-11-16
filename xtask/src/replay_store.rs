//! Exercise the server's bundle validation for every terminal run in a CI store.
use std::{collections::BTreeSet, path::Path};

use anyhow::{Context, Result, bail};
use duhem_evidence::{
    ReplayError, RunBundle, RunRecord, RunStatus, SqliteStore, Store, Trace, replay,
};

pub fn run(args: Vec<String>) -> Result<()> {
    let [db] = args.as_slice() else {
        bail!("usage: cargo run -p xtask -- replay-store <db>");
    };
    tokio::runtime::Runtime::new()?.block_on(check(Path::new(db)))
}

async fn check(db: &Path) -> Result<()> {
    let source = SqliteStore::open_read_only(db).await?;
    let scratch = tempfile::tempdir()?;
    let target = SqliteStore::open(scratch.path().join("ingest.db")).await?;
    check_runs(&source, &target, source.list_runs().await?).await
}

async fn check_runs(source: &dyn Store, target: &SqliteStore, runs: Vec<RunRecord>) -> Result<()> {
    let mut processed = BTreeSet::new();
    let mut ingested = BTreeSet::new();
    let mut finished = 0;
    let mut pending = 0;
    let mut skipped = 0;
    let mut blocked = 0;
    let mut divergences = 0;
    let mut tool_errors = 0;
    while processed.len() < runs.len() {
        let run = runs
            .iter()
            .find(|run| {
                !processed.contains(&run.run_id)
                    && run
                        .lineage
                        .parent_run_id
                        .as_ref()
                        .is_none_or(|parent| processed.contains(parent))
            })
            .context("replay-store tool error: missing parent or cyclic run lineage")?;
        processed.insert(run.run_id.clone());
        if run
            .lineage
            .parent_run_id
            .as_ref()
            .is_some_and(|parent| !ingested.contains(parent))
        {
            blocked += 1;
            continue;
        }
        let result = async {
            // Mirror duhem-cli/src/export_cmd.rs::ship_tree: pending logical
            // shard parents are eligible and ingest accepts their lone header.
            if matches!(run.status, RunStatus::Running | RunStatus::Orphaned) {
                return Ok(None);
            }
            let bundle = RunBundle::from_store(source, &run.run_id).await?;
            let hash = bundle.content_hash()?;
            match target.ingest_bundle(&bundle, &hash).await {
                Ok(()) => Ok(Some(Ok(()))),
                Err(error) => {
                    // Ingest wraps replay errors as strings. Classify via the typed
                    // replay result, never by matching a database error's wording.
                    let trace = Trace::from_events(bundle.events.clone())?;
                    if let Err(ReplayError::Divergence(divergence)) = replay(&trace) {
                        Ok(Some(Err(divergence)))
                    } else {
                        Err(anyhow::Error::from(error))
                    }
                }
            }
        }
        .await;
        match result {
            Ok(None) => skipped += 1,
            Ok(Some(Ok(()))) => {
                if run.status == RunStatus::Pending {
                    pending += 1;
                } else {
                    finished += 1;
                }
                ingested.insert(run.run_id.clone());
            }
            Ok(Some(Err(error))) => {
                divergences += 1;
                eprintln!(
                    "replay-store divergence: run {} verification {}: {error}",
                    run.run_id, run.verification
                );
            }
            Err(error) => {
                tool_errors += 1;
                eprintln!(
                    "replay-store tool error: run {} verification {}: {error:#}",
                    run.run_id, run.verification
                );
            }
        }
    }
    let summary = format!(
        "{divergences} divergent run(s), {tool_errors} tool error(s), {finished} finished verified, {pending} pending parents ingested, {skipped} unfinished skipped, {blocked} descendants blocked"
    );
    if divergences > 0 || tool_errors > 0 {
        bail!("replay-store: {summary}");
    }
    if !runs.is_empty() && finished == 0 {
        bail!("replay-store verified nothing: {summary}");
    }
    println!("replay-store: {summary}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::Arc};

    use duhem_evidence::{
        EventPayload, EvidenceWriter, RunLineage, RunOrigin, RunScope, SecretRegistry,
        VerdictState, run_started_with_definition_and_lineage,
    };

    async fn record(store: Arc<SqliteStore>, id: &str, parent: Option<&str>, tampered: bool) {
        let lineage = RunLineage {
            parent_run_id: parent.map(str::to_owned),
            origin: (id == "shard").then_some(RunOrigin::Shard),
        };
        let mut writer = EvidenceWriter::begin_scoped_with_secrets_and_lineage(
            store,
            id,
            "fixture.yml",
            BTreeMap::new(),
            RunScope::default(),
            lineage.clone(),
            SecretRegistry::new(),
        )
        .await
        .unwrap();
        writer
            .append(run_started_with_definition_and_lineage(
                "fixture.yml",
                BTreeMap::new(),
                None,
                lineage,
            ))
            .await
            .unwrap();
        if id == "unfinished" {
            return;
        }
        if id == "shard" {
            writer
                .append(EventPayload::ClaimScopeDeclared {
                    scope: Box::new(
                        serde_json::from_value(serde_json::json!({
                            "criteria": {"authored": 1, "selected": 1, "executed": 0},
                            "checks": {"authored": 1, "selected": 1, "executed": 0},
                            "shard": {"index": 1, "total": 2, "source": "env"},
                            "manifest_digest": "a".repeat(64),
                            "expected_partition": {"units": ["leaf"], "total_units": 2}
                        }))
                        .unwrap(),
                    ),
                })
                .await
                .unwrap();
        }
        writer
            .append(EventPayload::AssertionEvaluated {
                check_id: "AC-1.1".into(),
                diagnostic: false,
                operands: None,
                assertion_index: 0,
                iteration: None,
                state: VerdictState::Pass,
                detail: None,
                expr: Some("true".into()),
                step_index: None,
            })
            .await
            .unwrap();
        let verdict = if tampered {
            VerdictState::Fail
        } else {
            VerdictState::Pass
        };
        writer
            .append(EventPayload::CheckFinished {
                check_id: "AC-1.1".into(),
                criterion_id: Some("AC-1".into()),
                verdict,
                gated_judging_steps: 0,
                session_source: None,
                session_digest: None,
            })
            .await
            .unwrap();
        writer
            .append(EventPayload::CriterionFinished {
                criterion_id: "AC-1".into(),
                verdict,
            })
            .await
            .unwrap();
        writer
            .append(EventPayload::RunFinished {
                verdict: Some(verdict),
                body_verdict: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn clean_store_passes_and_tampered_check_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("source.db");
        let store = Arc::new(SqliteStore::open(&db).await.unwrap());
        record(store.clone(), "clean", None, false).await;
        record(store.clone(), "unfinished", None, false).await;
        check(&db).await.unwrap();
        record(store, "tampered", None, true).await;
        let error = check(&db).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("1 divergent run(s), 0 tool error(s)")
        );
    }

    #[tokio::test]
    async fn pending_logical_parent_allows_shard_and_leaf_verification() {
        let tmp = tempfile::tempdir().unwrap();
        let source = Arc::new(
            SqliteStore::open(tmp.path().join("source.db"))
                .await
                .unwrap(),
        );
        let target = SqliteStore::open(tmp.path().join("target.db"))
            .await
            .unwrap();
        record(source.clone(), "unfinished", None, false).await;
        record(source.clone(), "shard", Some("unfinished"), false).await;
        record(source.clone(), "leaf", Some("shard"), false).await;
        assert_eq!(
            source.get_run("unfinished").await.unwrap().unwrap().status,
            RunStatus::Pending
        );
        let mut runs = Vec::new();
        for id in ["leaf", "shard", "unfinished"] {
            runs.push(source.get_run(id).await.unwrap().unwrap());
        }
        check_runs(source.as_ref(), &target, runs).await.unwrap();
        assert_eq!(target.list_runs().await.unwrap().len(), 3);
        assert_eq!(
            target.get_run("leaf").await.unwrap().unwrap().verdict,
            Some(VerdictState::Pass)
        );
    }

    #[tokio::test]
    async fn nonempty_store_with_no_verified_runs_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("source.db");
        let source = Arc::new(SqliteStore::open(&db).await.unwrap());
        record(source, "unfinished", None, false).await;
        assert!(
            check(&db)
                .await
                .unwrap_err()
                .to_string()
                .contains("replay-store verified nothing")
        );
    }

    #[tokio::test]
    async fn reverse_order_tree_is_ingested_parent_first() {
        let tmp = tempfile::tempdir().unwrap();
        let source = Arc::new(
            SqliteStore::open(tmp.path().join("source.db"))
                .await
                .unwrap(),
        );
        let target = SqliteStore::open(tmp.path().join("target.db"))
            .await
            .unwrap();
        record(source.clone(), "parent", None, false).await;
        record(source.clone(), "child", Some("parent"), false).await;
        record(source.clone(), "grandchild", Some("child"), false).await;
        let mut runs = Vec::new();
        for id in ["grandchild", "child", "parent"] {
            runs.push(source.get_run(id).await.unwrap().unwrap());
        }
        check_runs(source.as_ref(), &target, runs).await.unwrap();
        assert_eq!(target.list_runs().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn store_failure_is_a_tool_error_not_a_divergence() {
        let tmp = tempfile::tempdir().unwrap();
        let source = Arc::new(
            SqliteStore::open(tmp.path().join("source.db"))
                .await
                .unwrap(),
        );
        record(source.clone(), "clean", None, false).await;
        let target_path = tmp.path().join("target.db");
        let _target = SqliteStore::open(&target_path).await.unwrap();
        let read_only = SqliteStore::open_read_only(&target_path).await.unwrap();
        let error = check_runs(
            source.as_ref(),
            &read_only,
            source.list_runs().await.unwrap(),
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("0 divergent run(s), 1 tool error(s)")
        );
    }

    #[tokio::test]
    async fn descendants_of_unfinished_and_rejected_parents_are_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let source = Arc::new(
            SqliteStore::open(tmp.path().join("source.db"))
                .await
                .unwrap(),
        );
        let target = SqliteStore::open(tmp.path().join("target.db"))
            .await
            .unwrap();
        record(source.clone(), "unfinished", None, false).await;
        record(source.clone(), "blocked-child", Some("unfinished"), false).await;
        record(source.clone(), "tampered", None, true).await;
        record(
            source.clone(),
            "blocked-grandchild",
            Some("tampered"),
            false,
        )
        .await;
        let error = check_runs(source.as_ref(), &target, source.list_runs().await.unwrap())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("1 divergent run(s), 0 tool error(s)")
        );
        assert!(
            error
                .to_string()
                .contains("1 unfinished skipped, 2 descendants blocked")
        );
        assert!(target.list_runs().await.unwrap().is_empty());
    }
}
