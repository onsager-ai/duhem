//! The evidence store: append-only run records for verification
//! evidence, persisted in a database (#189 — the store is the single
//! source of truth; per-run JSONL files are retired).
//!
//! Every action invocation, every assertion outcome, every artifact
//! the runtime touches gets recorded here so a verdict can be
//! independently reconstructed from the event stream alone — the
//! reproducibility floor under `docs/duhem-spec.md` §11.2's
//! mechanical-judgment commitment.
//!
//! Invariants (enforced by the store, not by convention):
//!
//! - The runtime is the **sole writer**; the judge stays a pure
//!   evaluator handing it verdicts. The dashboard reads through a
//!   read-only handle.
//! - Rows are **insert-only**; a run is sealed once its verdict
//!   lands. The `events` table is the wire-format stream (#10); the
//!   normalized tables are same-transaction projections of it.
//! - Portability is `duhem export` — a self-contained bundle of the
//!   run header, event stream, and artifacts.

pub mod event;
pub mod reader;
pub mod replay;
pub mod store;
pub mod writer;

pub use event::{
    BLOB_INLINE_THRESHOLD_BYTES, Event, EventPayload, ObservationValue, SCHEMA_VERSION,
    StepOutcome, VerdictState,
};
pub use reader::{ReadError, Trace};
pub use replay::{ReplayDivergence, ReplayError, ReplayedRun, replay};
pub use store::{
    CriterionHistoryEntry, ProjectSummary, RunMeta, RunRecord, RunScope, Span, SqliteStore, Store,
    StoreError, TargetStatus, project_db_path, project_slug, state_root, verification_name,
};
pub use writer::{EvidenceWriter, Sha256Hex, WriterError, run_started};

/// Generate a new run id — a ULID (sortable, opaque, time-prefixed).
pub fn new_run_id() -> String {
    ulid::Ulid::new().to_string()
}
