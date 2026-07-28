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
use crate::secret::SecretRegistry;
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
    /// Run-scoped secret spellings. Centralizing this on the writer is
    /// the sink-level guarantee from spec #346: callers append ordinary
    /// evidence and cannot accidentally opt an action out of masking.
    secrets: SecretRegistry,
    /// Counts produced when a blob was written, keyed by its content
    /// address. A subsequent observation reference picks them up without
    /// requiring capture/action call sites to carry masking metadata.
    artifact_mask_counts: BTreeMap<String, BTreeMap<String, u64>>,
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
        Self::begin_scoped_with_secrets(
            store,
            run_id,
            definition_path,
            inputs,
            RunScope::default(),
            SecretRegistry::new(),
        )
        .await
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
        Self::begin_scoped_with_secrets(
            store,
            run_id,
            definition_path,
            inputs,
            scope,
            SecretRegistry::new(),
        )
        .await
    }

    /// [`EvidenceWriter::begin_scoped`] with the run's resolved secret
    /// registry attached before the header row is written. The header,
    /// event stream, blobs, and post-commit tee therefore all see the
    /// same already-masked facts.
    pub async fn begin_scoped_with_secrets(
        store: Arc<dyn Store>,
        run_id: impl Into<String>,
        definition_path: &str,
        mut inputs: BTreeMap<String, serde_json::Value>,
        mut scope: RunScope,
        secrets: SecretRegistry,
    ) -> Result<Self, WriterError> {
        let run_id = run_id.into();
        for value in inputs.values_mut() {
            secrets.mask_json(value);
        }
        let definition_path = secrets.mask(definition_path).text;
        mask_optional(&secrets, &mut scope.project_id);
        mask_optional(&secrets, &mut scope.verifier_repo);
        mask_optional(&secrets, &mut scope.verifier_sha);
        mask_optional(&secrets, &mut scope.target_repo);
        mask_optional(&secrets, &mut scope.target_sha);
        store
            .begin_run(&RunMeta {
                run_id: run_id.clone(),
                verification: definition_path,
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
            secrets,
            artifact_mask_counts: BTreeMap::new(),
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

    /// Add a scalar value acquired during the run to this writer's
    /// secret registry (spec #355). Registration affects this point
    /// forward: events already committed remain unchanged, preserving
    /// append-only streaming rather than buffering for retroactive
    /// masking.
    pub fn register_secret(&mut self, source: impl Into<String>, value: &serde_json::Value) {
        self.secrets.register_json(source, value);
    }

    /// Apply the writer's current registry to a non-evidence text sink
    /// such as the structured run outcome rendered in the terminal.
    /// Keeping the registry private still makes the writer the single
    /// owner while letting the runtime honor the same boundary there.
    pub fn mask_text(&self, text: &str) -> String {
        self.secrets.mask(text).text
    }

    /// Append one event. The caller supplies the `payload`; `seq` and
    /// `ts` are stamped here.
    pub async fn append(&mut self, mut payload: EventPayload) -> Result<u64, WriterError> {
        self.attach_artifact_counts(&mut payload);
        // Round-trip the payload through its serde shape, masking every
        // textual leaf at this single sink. Event discriminants and
        // numeric lifecycle fields remain typed; dynamic `with:` and
        // observation values are traversed recursively.
        //
        // Skipped entirely when no secret is registered — the common
        // case, since a Verification Definition without `secret:` inputs
        // has nothing to mask. Beyond the wasted serialization, the
        // round-trip is the only way `append` can fail on a payload the
        // caller already built, and `EventPayload` carries an untagged
        // enum whose variant is reselected on the way back in. Neither
        // risk is worth taking for a registry that would replace nothing.
        if !self.secrets.is_empty() {
            let mut value = serde_json::to_value(&payload)?;
            self.secrets.mask_json(&mut value);
            payload = serde_json::from_value(value)?;
        }
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
            ObservationValue::Blob {
                mask_counts: self
                    .artifact_mask_counts
                    .get(sha.as_str())
                    .cloned()
                    .unwrap_or_default(),
                blob_sha256: sha.0,
            }
        } else {
            let mut value = value;
            self.secrets.mask_json(&mut value);
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
    /// Idempotent for identical masked content. UTF-8 blobs are recorded
    /// text and are masked here; binary blobs (including screenshots and
    /// video) remain byte-identical by the explicit spec #346 limit.
    pub async fn write_blob(&mut self, bytes: &[u8]) -> Result<Sha256Hex, WriterError> {
        let (masked, counts) = self.secrets.mask_bytes(bytes);
        let sha = self.store.put_blob(&masked).await?;
        if !counts.is_empty() {
            self.artifact_mask_counts.insert(sha.0.clone(), counts);
        }
        Ok(sha)
    }

    /// Close the writer. Every append already committed, so this is a
    /// consume-only marker — kept so call sites state intent (and so
    /// a future batching writer has a flush point).
    pub async fn finish(self) -> Result<(), WriterError> {
        Ok(())
    }

    fn attach_artifact_counts(&self, payload: &mut EventPayload) {
        let value = match payload {
            EventPayload::StepObservation { value, .. }
            | EventPayload::SetupStepObservation { value, .. } => value,
            _ => return,
        };
        if let ObservationValue::Blob {
            blob_sha256,
            mask_counts,
        } = value
            && mask_counts.is_empty()
            && let Some(counts) = self.artifact_mask_counts.get(blob_sha256)
        {
            *mask_counts = counts.clone();
        }
    }
}

fn mask_optional(secrets: &SecretRegistry, value: &mut Option<String>) {
    if let Some(value) = value {
        *value = secrets.mask(value).text;
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
