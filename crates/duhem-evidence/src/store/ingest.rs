//! Atomic replication of a validated v1 bundle through the normal projections.
use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::{BUNDLE_VERSION, EventPayload, RunBundle, RunLineage, RunMeta, RunScope, Trace};

use super::{SqliteStore, StoreError, sqlite};

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("invalid bundle: {0}")]
    Invalid(String),
    #[error("conflicting shard: {0}")]
    Conflict(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

fn invalid(error: impl std::fmt::Display) -> IngestError {
    IngestError::Invalid(error.to_string())
}

impl SqliteStore {
    /// Validate before writing; receipt, blobs, events and projections commit together.
    /// An unjudged container remains unjudged; judged bodies must replay exactly.
    pub async fn ingest_bundle(&self, bundle: &RunBundle, hash: &str) -> Result<(), IngestError> {
        if bundle.bundle_version != BUNDLE_VERSION
            || bundle.content_hash().map_err(invalid)? != hash
        {
            return Err(invalid("unsupported version or content hash mismatch"));
        }
        let run = &bundle.run;
        if run.run_id.is_empty() || run.parent_run_id.as_deref() == Some(run.run_id.as_str()) {
            return Err(invalid("invalid run identity"));
        }
        let trace = Trace::from_events(bundle.events.clone()).map_err(invalid)?;
        let Some(first) = trace.events().first() else {
            return Err(invalid("missing run_started"));
        };
        match &first.payload {
            EventPayload::RunStarted {
                verification_path,
                inputs,
                schema_version,
                parent_run_id,
                origin,
                ..
            } if verification_path == &run.verification
                && inputs == &run.inputs
                && schema_version == &run.schema_version
                && parent_run_id == &run.parent_run_id
                && origin == &run.origin => {}
            _ => return Err(invalid("run header disagrees with run_started")),
        }
        let terminals: Vec<_> = trace
            .events()
            .iter()
            .enumerate()
            .filter(|(_, event)| {
                matches!(
                    event.payload,
                    EventPayload::RunFinished { .. } | EventPayload::RunAborted { .. }
                )
            })
            .collect();
        if terminals.len() > 1
            || terminals
                .first()
                .is_some_and(|(index, _)| *index + 1 != trace.events().len())
        {
            return Err(invalid("terminal event must occur exactly once at the end"));
        }
        if terminals.is_empty()
            && (trace.events().len() != 1 || run.parent_run_id.is_some() || run.origin.is_some())
        {
            return Err(invalid("incomplete run"));
        }
        let judged = trace.events().iter().any(|event| {
            matches!(
                event.payload,
                EventPayload::RunFinished {
                    verdict: Some(_),
                    ..
                }
            )
        });
        if judged {
            // Replay recomputes check, criterion and run verdicts and rejects divergence.
            crate::replay(&trace).map_err(invalid)?;
        } else if trace
            .events()
            .iter()
            .filter(|event| matches!(event.payload, EventPayload::RunStarted { .. }))
            .count()
            != 1
            || trace.events().iter().any(|event| {
                matches!(
                    event.payload,
                    EventPayload::AssertionEvaluated { .. }
                        | EventPayload::CheckFinished { .. }
                        | EventPayload::CriterionFinished { .. }
                )
            })
        {
            return Err(invalid("unjudged container contains judgment evidence"));
        }
        let distributed = crate::claim_scope::claim_scope(&bundle.events)
            .map(|scope| scope.distributed)
            .unwrap_or_default();
        if run.shard != distributed.shard
            || run.manifest_digest != distributed.manifest_digest
            || run.expected_partition != distributed.expected_partition
        {
            return Err(invalid("distributed header disagrees with events"));
        }
        if run
            .shard
            .as_ref()
            .is_some_and(|shard| shard.index == 0 || shard.index > shard.total)
        {
            return Err(invalid("invalid shard index"));
        }
        if run.origin == Some(crate::RunOrigin::Shard)
            && (run.shard.is_none()
                || run.manifest_digest.is_none()
                || run.expected_partition.is_none())
        {
            return Err(invalid(
                "shard requires identity, manifest digest and expected partition",
            ));
        }
        let started_at: chrono::DateTime<chrono::Utc> = run.started_at.parse().map_err(invalid)?;
        let mut blobs = Vec::new();
        for artifact in &bundle.artifacts {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&artifact.bytes_base64)
                .map_err(invalid)?;
            if format!("{:x}", Sha256::digest(&bytes)) != artifact.sha256 {
                return Err(invalid("artifact hash mismatch"));
            }
            blobs.push((&artifact.sha256, bytes));
        }
        for sha in crate::bundle::referenced_blobs(&bundle.events) {
            if !blobs.iter().any(|(digest, _)| **digest == sha) {
                return Err(invalid("missing referenced artifact"));
            }
        }
        let meta = RunMeta {
            run_id: run.run_id.clone(),
            verification: run.verification.clone(),
            schema_version: run.schema_version.clone(),
            inputs: run.inputs.clone(),
            started_at,
            lineage: RunLineage {
                parent_run_id: run.parent_run_id.clone(),
                origin: run.origin,
            },
            scope: RunScope {
                project_id: run.project_id.clone(),
                verifier_repo: run.verifier_repo.clone(),
                verifier_sha: run.verifier_sha.clone(),
                target_repo: run.target_repo.clone(),
                target_sha: run.target_sha.clone(),
                branch: run.branch.clone(),
                pr: run.pr.clone(),
                ci_url: run.ci_url.clone(),
            },
        };
        // Acquire SQLite's writer reservation before checking the receipt, including
        // across independent server instances sharing the store.
        let mut tx = self
            .pool()
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(StoreError::from)?;
        let received: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM bundle_receipts WHERE content_hash = ?)",
        )
        .bind(hash)
        .fetch_one(&mut *tx)
        .await
        .map_err(StoreError::from)?;
        if received {
            return Ok(());
        }
        if run.origin == Some(crate::RunOrigin::Shard) {
            let shard = run.shard.as_ref().expect("validated shard identity");
            let siblings: Vec<String> = sqlx::query_scalar(
                "SELECT e.payload FROM events e JOIN runs r ON r.run_id = e.run_id \
                 WHERE r.parent_run_id = ? AND r.shard_origin = 1 AND e.kind = 'claim_scope_declared'",
            )
            .bind(&run.parent_run_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(StoreError::from)?;
            for payload in siblings {
                let event: crate::Event = serde_json::from_str(&payload).map_err(invalid)?;
                let EventPayload::ClaimScopeDeclared { scope } = event.payload else {
                    continue;
                };
                let previous = scope.distributed;
                if previous.manifest_digest != run.manifest_digest {
                    return Err(IngestError::Conflict(
                        "manifest_digest differs under logical parent".into(),
                    ));
                }
                if let Some(previous) = previous.shard {
                    if previous.total != shard.total {
                        return Err(IngestError::Conflict(
                            "shard total differs under logical parent".into(),
                        ));
                    }
                    if previous.index == shard.index {
                        return Err(IngestError::Conflict(format!(
                            "shard {} already has different content",
                            shard.index
                        )));
                    }
                }
            }
        }
        let existing: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE run_id = ?)")
                .bind(&run.run_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(StoreError::from)?;
        if existing {
            // Logical parents are sent independently by every shard. Their start
            // timestamps may differ; preserve the first unjudged container.
            let container = run.parent_run_id.is_none()
                && run.origin.is_none()
                && bundle.events.len() == 1
                && matches!(first.payload, EventPayload::RunStarted { .. });
            let unfinished: bool = sqlx::query_scalar(
                "SELECT NOT EXISTS(SELECT 1 FROM run_terminals WHERE run_id = ?) \
                 AND (SELECT COUNT(*) FROM events WHERE run_id = ?) = 1 \
                 AND EXISTS(SELECT 1 FROM runs WHERE run_id = ? AND parent_run_id IS NULL AND origin IS NULL)",
            )
            .bind(&run.run_id)
            .bind(&run.run_id)
            .bind(&run.run_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(StoreError::from)?;
            if !container || !unfinished {
                return Err(invalid("run already exists with different content"));
            }
        } else {
            if run.origin == Some(crate::RunOrigin::Shard) {
                let parent_id = run
                    .parent_run_id
                    .as_ref()
                    .ok_or_else(|| invalid("shard missing logical parent"))?;
                let parent_exists: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE run_id = ?)")
                        .bind(parent_id)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(StoreError::from)?;
                if !parent_exists {
                    let parent = RunMeta {
                        run_id: parent_id.clone(),
                        verification: run.verification.clone(),
                        schema_version: run.schema_version.clone(),
                        inputs: Default::default(),
                        started_at,
                        lineage: RunLineage::default(),
                        scope: meta.scope.clone(),
                    };
                    sqlite::begin_run(&mut tx, &parent).await?;
                    let event = crate::Event {
                        seq: 0,
                        ts: started_at,
                        session_source: None,
                        session_digest: None,
                        session: None,
                        payload: crate::run_started(run.verification.clone(), Default::default()),
                    };
                    sqlite::append_event(&mut tx, parent_id, &event).await?;
                }
            }
            sqlite::begin_run(&mut tx, &meta).await?;
            for (sha, bytes) in blobs {
                sqlx::query(
                    "INSERT OR IGNORE INTO artifacts (sha256, size, bytes) VALUES (?, ?, ?)",
                )
                .bind(sha)
                .bind(bytes.len() as i64)
                .bind(bytes)
                .execute(&mut *tx)
                .await
                .map_err(StoreError::from)?;
            }
            for event in &bundle.events {
                sqlite::append_event(&mut tx, &run.run_id, event).await?;
            }
        }
        sqlx::query(
            "INSERT INTO bundle_receipts (content_hash, run_id, received_at) VALUES (?, ?, ?)",
        )
        .bind(hash)
        .bind(&run.run_id)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(StoreError::from)?;
        tx.commit().await.map_err(StoreError::from)?;
        Ok(())
    }
}
