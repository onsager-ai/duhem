//! `duhem run --reporter` stdout formatters.
//!
//! Spec on issue #23. Three v1 reporters:
//!
//! - `default` — matches the pre-spec output byte-for-byte: a single
//!   line with the run verdict.
//! - `quiet` — no stdout; the exit code is the only signal.
//! - `json` — one JSON line: `{ run_id, verdict, criteria, evidence_dir }`.
//!
//! Reporters format post-run summary only. `trace.jsonl` is identical
//! regardless of the chosen reporter.

use std::io::Write;
use std::path::Path;

use duhem_judge::VerdictState;
use duhem_runtime::RunOutcome;
use serde::Serialize;

/// Selectable reporter. `clap::ValueEnum` is implemented in `main.rs`
/// to keep this module free of CLI dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reporter {
    Default,
    Quiet,
    Json,
}

/// Render the post-run summary for `outcome` to `out`. Reporter
/// selection is a stdout-only concern; the writer is parametric so
/// tests can capture output without going through the real stdout.
pub fn render(
    reporter: Reporter,
    out: &mut dyn Write,
    outcome: &RunOutcome,
) -> std::io::Result<()> {
    match reporter {
        Reporter::Default => writeln!(out, "{}", outcome.verdict.state),
        Reporter::Quiet => Ok(()),
        Reporter::Json => {
            let summary = JsonSummary::from_outcome(outcome);
            // One JSON object per run, newline-terminated. Authors
            // who want bulk-parsing get JSON-lines-friendly output.
            serde_json::to_writer(&mut *out, &summary)?;
            writeln!(out)
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonSummary<'a> {
    run_id: &'a str,
    verdict: VerdictState,
    criteria: Vec<JsonCriterion<'a>>,
    evidence_dir: &'a Path,
}

#[derive(Debug, Serialize)]
struct JsonCriterion<'a> {
    id: &'a str,
    verdict: VerdictState,
}

impl<'a> JsonSummary<'a> {
    fn from_outcome(o: &'a RunOutcome) -> Self {
        Self {
            run_id: &o.run_id,
            verdict: o.verdict.state,
            criteria: o
                .verdict
                .criteria
                .iter()
                .map(|c| JsonCriterion {
                    id: &c.criterion_id,
                    verdict: c.state,
                })
                .collect(),
            evidence_dir: &o.run_dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duhem_judge::{CheckVerdict, CriterionVerdict, InconclusiveCause, RunVerdict};
    use std::path::PathBuf;

    fn outcome(state: VerdictState) -> RunOutcome {
        RunOutcome {
            verdict: RunVerdict {
                state,
                criteria: vec![CriterionVerdict {
                    criterion_id: "AC-1".into(),
                    state,
                    checks: vec![CheckVerdict {
                        check_id: "AC-1.1".into(),
                        state,
                    }],
                }],
            },
            run_id: "01J000000000000000000RUN".into(),
            run_dir: PathBuf::from(".duhem/runs/01J000000000000000000RUN"),
        }
    }

    fn capture(reporter: Reporter, o: &RunOutcome) -> String {
        let mut buf = Vec::new();
        render(reporter, &mut buf, o).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn default_reporter_writes_single_verdict_line() {
        let s = capture(Reporter::Default, &outcome(VerdictState::Pass));
        // Byte-for-byte regression safety with the pre-spec output:
        // `println!("{}", verdict.state)` → "pass\n".
        assert_eq!(s, "pass\n");
    }

    #[test]
    fn default_reporter_emits_inconclusive_state() {
        let s = capture(
            Reporter::Default,
            &outcome(VerdictState::Inconclusive(InconclusiveCause::Timeout)),
        );
        assert_eq!(s, "inconclusive:timeout\n");
    }

    #[test]
    fn quiet_reporter_writes_nothing() {
        let s = capture(Reporter::Quiet, &outcome(VerdictState::Fail));
        assert_eq!(s, "");
    }

    #[test]
    fn json_reporter_is_single_line_valid_json() {
        let s = capture(Reporter::Json, &outcome(VerdictState::Pass));
        let trimmed = s.trim_end_matches('\n');
        assert!(!trimmed.contains('\n'), "single line: {s:?}");
        let v: serde_json::Value = serde_json::from_str(trimmed).expect("valid JSON");
        assert_eq!(v["run_id"], "01J000000000000000000RUN");
        assert_eq!(v["verdict"], "pass");
        assert_eq!(v["criteria"][0]["id"], "AC-1");
        assert_eq!(v["criteria"][0]["verdict"], "pass");
        assert_eq!(v["evidence_dir"], ".duhem/runs/01J000000000000000000RUN");
    }

    #[test]
    fn json_reporter_emits_inconclusive_wire_form() {
        let s = capture(
            Reporter::Json,
            &outcome(VerdictState::Inconclusive(
                InconclusiveCause::MissingObservation,
            )),
        );
        let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(v["verdict"], "inconclusive:missing_observation");
    }
}
