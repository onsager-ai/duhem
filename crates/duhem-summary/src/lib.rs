//! `RunSummary` — the versioned reporter-plugin contract (spec on
//! issue #34).
//!
//! Reporter plugins are subprocesses: the CLI writes one line of JSON
//! to the plugin's stdin and captures its stdout. The shape of that
//! line is `RunSummary` and it is **frozen**. Changes are
//! schema-impacting and require a `CHANGELOG.md` entry under the
//! `### Reporter contract` heading in the current `v0.x — unreleased`
//! section, plus a bump of [`RunSummary::SCHEMA_VERSION`].
//!
//! Phase-0 scope: criterion-level verdicts only. Per-check verdicts
//! stay in `trace.jsonl` (the trace is the trace; the summary is the
//! summary).
//!
//! The crate has exactly one dependency on `duhem-judge` (`VerdictState`)
//! so consumers — including reference plugins — can deserialize without
//! pulling in the CLI, runtime, or evidence layers.

use std::path::PathBuf;

use duhem_judge::VerdictState;
use serde::{Deserialize, Serialize};

/// One run's summary, serialized as one JSON line on the reporter
/// subprocess's stdin. The schema is the externally-frozen plugin
/// contract; field renames / removals are schema-impacting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunSummary {
    /// Always `"1"` at v1. Authored explicitly so a plugin written
    /// against today's contract can refuse to parse a future shape
    /// rather than silently misrender it.
    pub schema_version: String,
    /// Run identifier (the ULID created by `Engine::run_with_metadata`).
    pub run_id: String,
    /// Top-level verdict.
    pub verdict: VerdictState,
    /// Per-criterion verdicts, in document order.
    pub criteria: Vec<CriterionSummary>,
    /// On-disk evidence directory for the run.
    pub evidence_dir: PathBuf,
}

impl RunSummary {
    /// Current contract version. Bumping this is schema-impacting and
    /// requires a `CHANGELOG.md` entry under the
    /// `### Reporter contract` heading in the current
    /// `## v0.x — unreleased` section.
    pub const SCHEMA_VERSION: &'static str = "1";

    /// Construct a summary at the current schema version.
    pub fn new(
        run_id: impl Into<String>,
        verdict: VerdictState,
        criteria: Vec<CriterionSummary>,
        evidence_dir: PathBuf,
    ) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION.to_string(),
            run_id: run_id.into(),
            verdict,
            criteria,
            evidence_dir,
        }
    }
}

/// One criterion's verdict slot inside a `RunSummary`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CriterionSummary {
    pub id: String,
    pub verdict: VerdictState,
}

/// Aggregated summary across all leaves of a root-manifest run (spec
/// on issue #49). Shape is "the top-level verdict plus per-leaf
/// `RunSummary`s in execution order" — i.e. the same wire shape as a
/// reporter would see for a single leaf, lifted one level.
///
/// Version is decoupled from [`RunSummary::SCHEMA_VERSION`]: the
/// run-set wrapper is additive, so a plugin understanding `"1"` of
/// `RunSetSummary` necessarily understands the embedded `RunSummary`s'
/// own `schema_version`. Bumping either is schema-impacting and
/// requires a `CHANGELOG.md` entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunSetSummary {
    /// Always `"1"` at v1. See [`RunSummary::schema_version`] for the
    /// rationale.
    pub schema_version: String,
    /// Aggregated verdict per `aggregate_run_set`.
    pub verdict: VerdictState,
    /// Per-leaf summaries, in execution order.
    pub runs: Vec<RunSummary>,
}

impl RunSetSummary {
    /// Current contract version. Bumping this is schema-impacting and
    /// requires a `CHANGELOG.md` entry under the
    /// `### Reporter contract` heading.
    pub const SCHEMA_VERSION: &'static str = "1";

    pub fn new(verdict: VerdictState, runs: Vec<RunSummary>) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION.to_string(),
            verdict,
            runs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let s = RunSummary::new(
            "01J000000000000000000RUN",
            VerdictState::Pass,
            vec![CriterionSummary {
                id: "AC-1".into(),
                verdict: VerdictState::Pass,
            }],
            PathBuf::from(".duhem/runs/01J000000000000000000RUN"),
        );
        let line = serde_json::to_string(&s).unwrap();
        let back: RunSummary = serde_json::from_str(&line).unwrap();
        assert_eq!(back, s);
        // Sanity: the contract is versioned on the wire, not just in
        // memory — a plugin sees `schema_version` as a field.
        assert!(line.contains("\"schema_version\":\"1\""), "got: {line}");
    }

    #[test]
    fn schema_version_constant_matches_runtime() {
        let s = RunSummary::new("x", VerdictState::Pass, vec![], PathBuf::from("."));
        assert_eq!(s.schema_version, RunSummary::SCHEMA_VERSION);
    }

    #[test]
    fn run_set_summary_round_trips_through_json() {
        let leaf = RunSummary::new(
            "01J00000000000000000000001",
            VerdictState::Pass,
            vec![CriterionSummary {
                id: "AC-1".into(),
                verdict: VerdictState::Pass,
            }],
            PathBuf::from(".duhem/runs/leaf-a/01J00000000000000000000001"),
        );
        let set = RunSetSummary::new(VerdictState::Pass, vec![leaf]);
        let line = serde_json::to_string(&set).unwrap();
        let back: RunSetSummary = serde_json::from_str(&line).unwrap();
        assert_eq!(back, set);
        assert!(line.contains("\"schema_version\":\"1\""));
        assert!(line.contains("\"runs\""));
    }
}
