//! Wire shapes for the dashboard JSON API.
//!
//! The contract lives in the planning-resolution comment on issue #53;
//! these structs are its Rust rendering. The dashboard is a *view*
//! over evidence: every verdict here is the judge's recorded verdict
//! from `trace.jsonl` (or, for run-set rollups, the judge's
//! `aggregate_run_set` fold over recorded child verdicts) — nothing is
//! re-judged at the view layer.

use chrono::{DateTime, Utc};
use duhem_evidence::{Event, VerdictState};
use serde::Serialize;

/// Discriminates the two row kinds on the runs list. A `run-set` row
/// is a verification directory grouping leaf runs (#49 manifest runs
/// land at `<evidence-dir>/<leaf-name>/<run-id>/`); a `leaf` row is a
/// single run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryKind {
    Leaf,
    RunSet,
}

/// One row of `GET /api/runs`.
#[derive(Debug, Clone, Serialize)]
pub struct RunsListEntry {
    /// Leaf rows: the run's ULID. Run-set rows: the verification
    /// directory name (stable across requests; not addressable via
    /// `/api/runs/:run_id`).
    pub run_id: String,
    pub verification: String,
    pub started_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    /// `None` while the run is still in progress (#84): there is no
    /// verdict until the judge's `run_finished` lands in the trace.
    pub verdict: Option<VerdictState>,
    pub kind: EntryKind,
    /// `true` iff the trace has no `run_finished` yet (#84's "● live"
    /// affordance). Always `false` in static exports.
    pub live: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<RunsListEntry>>,
}

/// `GET /api/runs/:run_id`.
#[derive(Debug, Clone, Serialize)]
pub struct RunDetail {
    pub run_id: String,
    pub verification: String,
    pub started_at: Option<DateTime<Utc>>,
    pub inputs: serde_json::Map<String, serde_json::Value>,
    pub verdict: Option<VerdictState>,
    pub live: bool,
    /// `true` when the trace carries `setup_finished { aborted: true }`
    /// (#20) — the run never reached its checks.
    pub setup_aborted: bool,
    pub criteria: Vec<CriterionDetail>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CriterionDetail {
    pub id: String,
    pub verdict: Option<VerdictState>,
    pub checks: Vec<CheckRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckRef {
    pub id: String,
    pub verdict: Option<VerdictState>,
}

/// `GET /api/runs/:run_id/checks/:crit::check`.
#[derive(Debug, Clone, Serialize)]
pub struct CheckDetail {
    pub criterion_id: String,
    pub check_id: String,
    pub verdict: Option<VerdictState>,
    /// The check's slice of the trace, in trace order: `step_started`,
    /// `step_observation`, `step_finished`, `assertion_evaluated`,
    /// `check_finished`. Events are rendered as-is (same wire shape as
    /// `trace.jsonl` lines) — the trace is the truth, the timeline is
    /// a filter over it.
    pub timeline: Vec<Event>,
    pub artifacts: Vec<ArtifactRef>,
}

/// A content-addressed blob referenced from the check's timeline.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactRef {
    /// The blob's sha-256 (the artifact id in `/artifact/:id` URLs).
    pub id: String,
    /// The observation's `output_name` (e.g. `body`, `stdout`) — what
    /// the step called this artifact, not a media type.
    pub kind: String,
    /// Where to fetch the bytes. Serve mode: the API artifact route.
    /// Static export rewrites this to the exported relative path.
    pub url: String,
}
