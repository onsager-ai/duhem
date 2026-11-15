//! `duhem-dashboard` — read-only web dashboard over the Duhem
//! evidence store (specs #53 / #85 / #86 / #87, live streaming #84;
//! store posture #189).
//!
//! Reader routes open a read-only handle and display recorded evidence.
//! With explicit `--ingest` opt-in (#650), a separate writable handle
//! replicates validated bundles after replaying their judged events;
//! it never executes verifications. The store remains the single source
//! of truth, and the normal event projections serve both local and
//! replicated evidence. Default serve and export modes stay read-only.
//!
//! Three surfaces:
//! - [`server::router`] — the JSON API + embedded SPA (serve mode);
//! - [`live::live_stream`] — replay-then-follow SSE over a run's
//!   event stream (#84);
//! - [`export::export`] — a self-contained static rendering for
//!   upload from CI (#87).

pub mod export;
pub mod ingest;
pub mod live;
pub mod model;
pub mod reader;
pub mod server;

pub use export::{ExportStats, export};
pub use reader::{EvidenceReader, ReaderError, RunEvidence, events_to_jsonl};
pub use server::router;

/// Default listen port (#53 alignment: bikeshed-accepted 7878).
pub const DEFAULT_PORT: u16 = 7878;
