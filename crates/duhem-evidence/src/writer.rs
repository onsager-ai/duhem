//! Append-only writer for a run's event stream, backed by the store
//! (#189 — the JSONL file writer this replaces lived here until then).
//!
//! Contract (carried over from issue #10, now enforced by the DB):
//!
//! - One writer per run; the runtime owns it. The writer stamps `seq`
//!   and `ts`, so monotonicity is the writer's responsibility, not
//!   the caller's.
//! - Every append is a committed transaction — durability is at least
//!   as strong as the old fsync-on-`*_finished` policy.
//! - Blobs are content-addressed (`sha256`) in the store's `artifacts`
//!   table; puts are idempotent.
//! - Appending `run_finished` seals the run: the store folds the
//!   verdict row in the same transaction and rejects any later event.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, SubsecRound, Utc};
use thiserror::Error;

use crate::event::{
    BLOB_INLINE_THRESHOLD_BYTES, Event, EventPayload, ObservationValue, SCHEMA_VERSION,
};
use crate::store::{RunMeta, RunScope, Store, StoreError};

/// Truncate to millisecond precision. The wire format pins `ts` at
/// ms; in-memory `Utc::now()` carries ns. Truncate at the stamping
/// boundary so the value matches the wire form exactly.
fn now_ms() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(3)
}

#[derive(Debug, Error)]
pub enum WriterError {
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error("serialize error: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// SHA-256 digest of a blob, as lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Hex(pub String);

impl Sha256Hex {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Append-only writer for a single run.
pub struct EvidenceWriter {
    store: Arc<dyn Store>,
    run_id: String,
    next_seq: u64,
    /// Optional live tee (#299): every successfully persisted event is
    /// also sent here, stamped, in `seq` order. This is the runtime's
    /// in-process progress seam — a live terminal renderer (or any
    /// same-process observer) subscribes without polling the store.
    /// Send failures are ignored: a dropped receiver must never affect
    /// the run or the evidence.
    tee: Option<tokio::sync::mpsc::UnboundedSender<Event>>,
}

impl EvidenceWriter {
    /// Register the run with the store and open a writer for it.
    ///
    /// `definition_path` + `inputs` land in the run header row (the
    /// `manifest.json` successor); the caller still emits the
    /// `run_started` event (with the same facts) as its first
    /// `append` — the event stream stays self-describing on export.
    pub async fn begin(
        store: Arc<dyn Store>,
        run_id: impl Into<String>,
        definition_path: &str,
        inputs: BTreeMap<String, serde_json::Value>,
    ) -> Result<Self, WriterError> {
        Self::begin_scoped(store, run_id, definition_path, inputs, RunScope::default()).await
    }

    /// [`EvidenceWriter::begin`] with scoping + provenance (#190):
    /// the project hint and the `verifier VERIFIES target`
    /// coordinates land on the run header row.
    pub async fn begin_scoped(
        store: Arc<dyn Store>,
        run_id: impl Into<String>,
        definition_path: &str,
        inputs: BTreeMap<String, serde_json::Value>,
        scope: RunScope,
    ) -> Result<Self, WriterError> {
        let run_id = run_id.into();
        store
            .begin_run(&RunMeta {
                run_id: run_id.clone(),
                verification: definition_path.to_string(),
                schema_version: SCHEMA_VERSION.to_string(),
                inputs,
                started_at: now_ms(),
                scope,
            })
            .await?;
        Ok(Self {
            store,
            run_id,
            next_seq: 0,
            tee: None,
        })
    }

    /// Attach a live tee (#299): every event appended from here on is
    /// also sent to `tx` after it committed to the store. Evidence
    /// stays the single source of truth — the tee only ever sees what
    /// the store already accepted.
    pub fn with_tee(mut self, tx: tokio::sync::mpsc::UnboundedSender<Event>) -> Self {
        self.tee = Some(tx);
        self
    }

    /// The run this writer is appending to.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// The store this writer appends to (for read-back within the
    /// same process, e.g. the CLI rendering a run it just executed).
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Append one event. The caller supplies the `payload`; `seq` and
    /// `ts` are stamped here.
    pub async fn append(&mut self, payload: EventPayload) -> Result<u64, WriterError> {
        let evt = Event {
            seq: self.next_seq,
            ts: now_ms(),
            payload,
        };
        self.store.append_event(&self.run_id, &evt).await?;
        // Tee after the commit (#299): observers only see persisted
        // events, and a gone receiver is silently ignored.
        if let Some(tx) = &self.tee {
            let _ = tx.send(evt);
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        Ok(seq)
    }

    /// Convenience: emit a `step_observation`, choosing inline vs
    /// blob automatically based on the serialized byte length of
    /// `value` against [`BLOB_INLINE_THRESHOLD_BYTES`].
    pub async fn append_observation(
        &mut self,
        step_index: u32,
        output_name: impl Into<String>,
        value: serde_json::Value,
    ) -> Result<u64, WriterError> {
        let inline_bytes = serde_json::to_vec(&value)?;
        let obs = if inline_bytes.len() > BLOB_INLINE_THRESHOLD_BYTES {
            let sha = self.write_blob(&inline_bytes).await?;
            ObservationValue::Blob { blob_sha256: sha.0 }
        } else {
            ObservationValue::Inline { value }
        };
        self.append(EventPayload::StepObservation {
            step_index,
            output_name: output_name.into(),
            value: obs,
        })
        .await
    }

    /// Store a content-addressed blob and return its address.
    /// Idempotent for identical content.
    pub async fn write_blob(&mut self, bytes: &[u8]) -> Result<Sha256Hex, WriterError> {
        Ok(self.store.put_blob(bytes).await?)
    }

    /// Close the writer. Every append already committed, so this is a
    /// consume-only marker — kept so call sites state intent (and so
    /// a future batching writer has a flush point).
    pub async fn finish(self) -> Result<(), WriterError> {
        Ok(())
    }
}

/// Helper for building a `run_started` payload without hand-rolling
/// `BTreeMap` everywhere. Records no definition snapshot (used by tests
/// and any caller without the source in hand); the real run path uses
/// [`run_started_with_definition`].
pub fn run_started(
    verification_path: impl Into<String>,
    inputs: BTreeMap<String, serde_json::Value>,
) -> EventPayload {
    run_started_with_definition(verification_path, inputs, None)
}

/// [`run_started`] carrying the Verification Definition source snapshot
/// (spec #302) — the raw YAML the run was judged against, so evidence is
/// self-describing. `None` records no snapshot (backward compatible).
pub fn run_started_with_definition(
    verification_path: impl Into<String>,
    inputs: BTreeMap<String, serde_json::Value>,
    definition: Option<String>,
) -> EventPayload {
    EventPayload::RunStarted {
        verification_path: verification_path.into(),
        inputs,
        schema_version: SCHEMA_VERSION.to_string(),
        definition,
    }
}
