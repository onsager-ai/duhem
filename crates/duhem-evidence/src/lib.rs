//! Append-only run trace + blob writer for verification evidence.
//!
//! Every action invocation, every assertion outcome, every artifact
//! the runtime touches gets recorded here so a verdict can be
//! independently reconstructed from the trace alone — the
//! reproducibility floor under `docs/duhem-spec.md` §11.2's
//! mechanical-judgment commitment.
//!
//! Append-only; never mutated after `run_finished`. On-disk layout
//! (issue #10):
//!
//! ```text
//! .duhem/runs/<run_id>/
//!   trace.jsonl          # structured events, one JSON object per line
//!   blobs/<sha256>       # content-addressed binaries
//!   manifest.json        # run-level header
//! ```

pub mod event;
pub mod reader;
pub mod replay;
pub mod writer;

pub use event::{
    AssertionState, BLOB_INLINE_THRESHOLD_BYTES, Event, EventPayload, ObservationValue,
    SCHEMA_VERSION, StepOutcome, Verdict,
};
pub use reader::{ReadError, Trace};
pub use replay::{ReplayDivergence, ReplayError, RunVerdict, replay};
pub use writer::{EvidenceWriter, Manifest, Sha256Hex, WriterError, run_started};

/// Generate a new run id — a ULID (sortable, opaque, time-prefixed).
/// The directory under `.duhem/runs/` is named with this string.
pub fn new_run_id() -> String {
    ulid::Ulid::new().to_string()
}
