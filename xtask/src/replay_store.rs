//! Exercise the server's bundle validation for every terminal run in a CI store.
use std::path::Path;

use anyhow::{Result, bail};
use duhem_evidence::{EventPayload, RunBundle, SqliteStore, Store};

pub fn run(args: Vec<String>) -> Result<()> {
    let [db] = args.as_slice() else {
        bail!("usage: cargo run -p xtask -- replay-store <db>");
    };
    tokio::runtime::Runtime::new()?.block_on(check(Path::new(db)))
}

async fn check(db: &Path) -> Result<()> {
    let source = SqliteStore::open_read_only(db).await?;
    let scratch = tempfile::tempdir()?;
    let mut finished = 0;
    let mut skipped = 0;
    let mut divergences = 0;
    for run in source.list_runs().await? {
        let events = source.run_events(&run.run_id).await?;
        if !events.iter().any(|event| {
            matches!(
                event.payload,
                EventPayload::RunFinished { .. } | EventPayload::RunAborted { .. }
            )
        }) {
            skipped += 1;
            continue;
        }
        finished += 1;
        let result = async {
            let bundle = RunBundle::from_store(&source, &run.run_id).await?;
            // Independent targets avoid treating distinct runs from fixture
            // scenarios as replication conflicts. All ingest validation still runs.
            let target = SqliteStore::open(scratch.path().join(format!("{finished}.db"))).await?;
            target
                .ingest_bundle(&bundle, &bundle.content_hash()?)
                .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            eprintln!(
                "replay-store: run {} verification {}: {error:#}",
                run.run_id, run.verification
            );
            divergences += 1;
        }
    }
    if divergences > 0 {
        bail!(
            "replay-store: {divergences} divergent run(s), {finished} finished, {skipped} unfinished skipped"
        );
    }
    println!("replay-store: {finished} finished run(s) verified, {skipped} unfinished skipped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::Arc};

    use duhem_evidence::{EvidenceWriter, VerdictState, run_started};

    async fn record(store: Arc<SqliteStore>, id: &str, tampered: bool) {
        let mut writer = EvidenceWriter::begin(store, id, "fixture.yml", BTreeMap::new())
            .await
            .unwrap();
        writer
            .append(run_started("fixture.yml", BTreeMap::new()))
            .await
            .unwrap();
        if id == "unfinished" {
            return;
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
        record(store.clone(), "clean", false).await;
        record(store.clone(), "unfinished", false).await;
        check(&db).await.unwrap();
        record(store, "tampered", true).await;
        let error = check(&db).await.unwrap_err();
        assert!(error.to_string().contains("1 divergent run(s)"));
    }
}
