//! `duhem-dashboard` — read-only web dashboard over Duhem run
//! evidence (specs #53 / #85 / #86 / #87, live streaming #84).
//!
//! The dashboard is a *view* over `.duhem/runs/` evidence: it owns no
//! mutable state, never invokes the runtime or judge, and every
//! verdict it shows is the judge's recorded verdict from
//! `trace.jsonl`. That posture is the identity cross-check from
//! spec #53 — the evidence is the truth, the dashboard is a lens.
//!
//! Three surfaces:
//! - [`server::router`] — the JSON API + embedded SPA (serve mode);
//! - [`live::live_stream`] — replay-then-follow SSE over a run's
//!   trace (#84);
//! - [`export::export`] — a self-contained static rendering for
//!   upload from CI (#87).

pub mod export;
pub mod live;
pub mod model;
pub mod reader;
pub mod server;

pub use export::{ExportStats, export};
pub use reader::{EvidenceReader, ReaderError, RunEvidence, load_run};
pub use server::router;

/// Default listen port (#53 alignment: bikeshed-accepted 7878).
pub const DEFAULT_PORT: u16 = 7878;
/// Default evidence directory, matching the engine's default root.
pub const DEFAULT_EVIDENCE_DIR: &str = ".duhem/runs";
