//! `duhem-dashboard` — read-only web dashboard over the Duhem
//! evidence store (specs #53 / #85 / #86 / #87, live streaming #84;
//! store posture #189).
//!
//! The dashboard is a *lens* over the store: it opens a **read-only**
//! handle, owns no mutable state, never invokes the runtime or judge,
//! and every verdict it shows is the judge's recorded verdict. That
//! posture is the identity cross-check from spec #53, restated for
//! the store era on #189 — the store is the single source of truth,
//! the judge (via the runtime) is its sole writer, the dashboard is a
//! read-only lens, and `duhem export` is the portability path.
//!
//! Three surfaces:
//! - [`server::router`] — the JSON API + embedded SPA (serve mode);
//! - [`live::live_stream`] — replay-then-follow SSE over a run's
//!   event stream (#84);
//! - [`export::export`] — a self-contained static rendering for
//!   upload from CI (#87).

pub mod export;
pub mod live;
pub mod model;
pub mod reader;
pub mod server;

pub use export::{ExportStats, export};
pub use reader::{EvidenceReader, ReaderError, RunEvidence, events_to_jsonl};
pub use server::router;

/// Default listen port (#53 alignment: bikeshed-accepted 7878).
pub const DEFAULT_PORT: u16 = 7878;
