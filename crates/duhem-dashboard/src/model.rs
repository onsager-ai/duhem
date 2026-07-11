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
/// group several recorded runs); a `leaf` row is a
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
    /// The ordered delivery-web layers this check crossed (#192 data,
    /// ④ view). Empty for pre-tag runs / untagged steps — the view
    /// renders "layer unknown" rather than guessing.
    pub spans: Vec<SpanModel>,
    /// The check's slice of the trace, in trace order: `step_started`,
    /// `step_observation`, `step_finished`, `assertion_evaluated`,
    /// `check_finished`. Events are rendered as-is (same wire shape as
    /// `trace.jsonl` lines) — the trace is the truth, the timeline is
    /// a filter over it.
    pub timeline: Vec<Event>,
    pub artifacts: Vec<ArtifactRef>,
}

/// One delivery-web span (④): a layer the check crossed, with the
/// executed step's outcome. `seq` links back to the opening
/// `step_started` event on the timeline.
#[derive(Debug, Clone, Serialize)]
pub struct SpanModel {
    pub seq: u64,
    pub layer: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `GET /api/verifications/:name/history` — the ② VD-over-time shape:
/// the verification's runs (newest first) and each criterion as a
/// stable spine with its verdict on every run it appeared in.
#[derive(Debug, Clone, Serialize)]
pub struct VerificationHistory {
    pub name: String,
    /// Newest first — the column axis of the spine table. Ordered by
    /// `started_at` (the #190 history queries' axis).
    pub runs: Vec<HistoryRun>,
    /// First-seen order across runs; the row axis.
    pub criteria: Vec<CriterionHistory>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryRun {
    pub run_id: String,
    pub started_at: Option<DateTime<Utc>>,
    pub verdict: Option<VerdictState>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CriterionHistory {
    pub criterion_id: String,
    /// One entry per run in `runs` order; `None` = the criterion did
    /// not appear on that run (VD edited between runs).
    pub verdicts: Vec<Option<VerdictState>>,
}

/// `GET /api/runs/:run_id/diff` (#211): the run compared against its
/// baseline — the most recent prior *passing* run of the same
/// verification + target (last-pass). The diff is evidence, never a
/// judge input: it only surfaces recorded transitions, it never
/// recomputes a verdict.
#[derive(Debug, Clone, Serialize)]
pub struct RunDiff {
    pub current: RunSide,
    /// `None` when the verification has no prior passing run to compare
    /// against — the view says "no passing baseline" rather than
    /// diffing two failures.
    pub baseline: Option<RunSide>,
    pub criteria: Vec<CriterionDiff>,
}

/// One end of a diff: a run's identity + recorded verdict.
#[derive(Debug, Clone, Serialize)]
pub struct RunSide {
    pub run_id: String,
    pub started_at: Option<DateTime<Utc>>,
    pub verdict: Option<VerdictState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CriterionDiff {
    pub id: String,
    pub baseline_verdict: Option<VerdictState>,
    pub current_verdict: Option<VerdictState>,
    /// `true` iff a baseline exists and the verdict differs — the view
    /// surfaces changed criteria first.
    pub changed: bool,
    pub checks: Vec<CheckDiff>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckDiff {
    pub id: String,
    pub baseline_verdict: Option<VerdictState>,
    pub current_verdict: Option<VerdictState>,
    pub changed: bool,
    pub assertions: Vec<AssertionDiff>,
    /// The check's `capture/*` (and other blob) artifacts on each side,
    /// so the view can render baseline↔current evidence side by side
    /// and diff the HAR/screenshot itself.
    pub baseline_artifacts: Vec<ArtifactRef>,
    pub current_artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssertionDiff {
    pub assertion_index: u32,
    pub baseline_state: Option<VerdictState>,
    pub current_state: Option<VerdictState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_detail: Option<String>,
    pub changed: bool,
}

/// `GET /api/runs/:run_id/failure` (#216): the machine-readable
/// failure envelope for an agent reacting to a `fail` in CI —
/// everything needed to close the verify→repair loop without scraping
/// the SPA. Derived mechanically from the recorded trace; never a
/// judge input, no verdict recomputed. This is an agent-facing
/// contract (`docs/failure-envelope-contract.md`).
#[derive(Debug, Clone, Serialize)]
pub struct FailureEnvelope {
    pub run_id: String,
    pub verification: String,
    pub verdict: Option<VerdictState>,
    /// One entry per non-passing check. Empty on a fully-passing run.
    pub failing: Vec<FailingCheck>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailingCheck {
    pub criterion_id: String,
    pub check_id: String,
    pub verdict: Option<VerdictState>,
    /// The delivery-web layer chain the check crossed (#192), in order
    /// — `ui` / `api` / `data` / `runtime`. Empty for pre-tag runs.
    pub layers: Vec<String>,
    /// The non-passing assertions with their recorded cause.
    pub assertions: Vec<FailingAssertion>,
    /// The check's `capture/*` (and other blob) artifacts.
    pub artifacts: Vec<ArtifactRef>,
    /// The first request in this check's captured network (#204) whose
    /// status is an error (≥ 400) — usually the request that broke.
    /// `None` when there's no network capture or no error response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_failing_request: Option<FailingRequest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailingAssertion {
    pub assertion_index: u32,
    pub state: VerdictState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailingRequest {
    pub method: String,
    pub url: String,
    pub status: u16,
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
