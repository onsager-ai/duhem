//! Reference plugin reporter: JUnit XML over the `RunSummary` plugin
//! contract (spec on issue #34).
//!
//! Mapping (deliberately minimal — JUnit consumers vary on what they
//! accept, so we stick to the subset every consumer parses):
//!
//! - One `<testsuite>` per run.
//! - One `<testcase>` per criterion, and suite `tests` / `failures` /
//!   `skipped` attributes that count those same elements. JUnit
//!   consumers cross-check the attributes against the `<testcase>`
//!   children, so the two must describe the same population — this is
//!   deliberately *not* `RunSummary.totals`, which counts checks (#493).
//!   The check-level aggregate is available on the JSON reporter.
//! - `pass` → empty testcase.
//! - `fail` → `<testcase><failure type="fail"/></testcase>`.
//! - `inconclusive:<cause>` → `<testcase><skipped type="<cause>"/></testcase>`.
//!
//! Exits 0 on success, 2 on parse / schema-version failure (mirrors
//! the `pretty` reference plugin).

use std::io::{self, Read, Write};

use duhem_judge::VerdictState;
use duhem_summary::{LifecyclePhase, LifecycleStatus, RunSummary};

fn main() {
    let mut buf = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut buf) {
        eprintln!("duhem-reporter-junit: read stdin: {e}");
        std::process::exit(2);
    }
    let line = buf.trim().lines().next().unwrap_or("");
    let summary: RunSummary = match serde_json::from_str(line) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("duhem-reporter-junit: parse RunSummary: {e}");
            std::process::exit(2);
        }
    };

    if summary.schema_version != RunSummary::SCHEMA_VERSION {
        eprintln!(
            "duhem-reporter-junit: unsupported RunSummary schema_version `{}` (this plugin understands `{}`)",
            summary.schema_version,
            RunSummary::SCHEMA_VERSION,
        );
        std::process::exit(2);
    }

    let xml = render(&summary);
    // Use `write_all` rather than `print!` so a closed downstream
    // pipe surfaces as a tidy exit-2 with a recognizable message,
    // not a Rust panic / exit 101.
    if let Err(e) = io::stdout().write_all(xml.as_bytes()) {
        eprintln!("duhem-reporter-junit: write stdout: {e}");
        std::process::exit(2);
    }
}

/// Render the `RunSummary` as a JUnit XML document. Returned as a
/// `String` so the test below can assert on the exact wire shape
/// without going through stdout.
fn render(s: &RunSummary) -> String {
    // Counts describe the `<testcase>` elements emitted below, which are
    // criteria. Substituting `RunSummary.totals` here would claim N
    // checks while emitting one element per criterion, which parsers
    // read as a malformed suite (#493).
    let lifecycle_failures = s
        .lifecycle
        .iter()
        .filter(|block| block.status != LifecycleStatus::Passed)
        .count();
    let total = s.criteria.len() + lifecycle_failures;
    let failures = s
        .criteria
        .iter()
        .filter(|c| matches!(c.verdict, VerdictState::Fail))
        .count()
        + lifecycle_failures;
    let skipped = s
        .criteria
        .iter()
        .filter(|c| matches!(c.verdict, VerdictState::Inconclusive(_)))
        .count();

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuite name=\"{}\" tests=\"{total}\" failures=\"{failures}\" skipped=\"{skipped}\">\n",
        xml_escape(&s.run_id),
    ));
    for c in &s.criteria {
        match &c.verdict {
            VerdictState::Pass => {
                out.push_str(&format!("  <testcase name=\"{}\"/>\n", xml_escape(&c.id)));
            }
            VerdictState::Fail => {
                out.push_str(&format!(
                    "  <testcase name=\"{}\"><failure type=\"fail\"/></testcase>\n",
                    xml_escape(&c.id)
                ));
            }
            VerdictState::Inconclusive(cause) => {
                // `cause` derefs to the wire-form lowercase name via
                // `VerdictState::Display`; strip the `inconclusive:`
                // prefix so the `type` attribute is just the cause.
                let cause_wire = format!("{}", VerdictState::Inconclusive(*cause));
                let cause_type = cause_wire
                    .strip_prefix("inconclusive:")
                    .unwrap_or(&cause_wire);
                out.push_str(&format!(
                    "  <testcase name=\"{}\"><skipped type=\"{}\"/></testcase>\n",
                    xml_escape(&c.id),
                    xml_escape(cause_type),
                ));
            }
        }
    }
    for block in s
        .lifecycle
        .iter()
        .filter(|block| block.status != LifecycleStatus::Passed)
    {
        let name = format!("{} {}", block.scope_path(), phase_label(block.phase));
        let detail = block
            .failing_step
            .and_then(|position| block.steps.get(position).map(|step| (position, step)))
            .map(|(position, step)| {
                let suffix = step
                    .detail
                    .as_deref()
                    .map(|detail| format!(": {detail}"))
                    .unwrap_or_default();
                format!(
                    "failing step {position}: {} #{}{}",
                    step.uses, step.index, suffix
                )
            })
            .unwrap_or_else(|| format!("{} lifecycle block", status_label(block.status)));
        out.push_str(&format!(
            "  <testcase name=\"{}\"><failure type=\"lifecycle\">{}</failure></testcase>\n",
            xml_escape(&name),
            xml_escape(&detail),
        ));
    }
    let passed_lifecycle = s
        .lifecycle
        .iter()
        .filter(|block| block.status == LifecycleStatus::Passed)
        .map(|block| format!("{} {} passed", block.scope_path(), phase_label(block.phase)))
        .collect::<Vec<_>>();
    if !passed_lifecycle.is_empty() {
        out.push_str(&format!(
            "  <system-out>{}</system-out>\n",
            xml_escape(&passed_lifecycle.join("\n")),
        ));
    }
    if !s.cleanup.is_empty() {
        let detail = s
            .cleanup
            .iter()
            .map(|failure| {
                let suffix = failure
                    .detail
                    .as_deref()
                    .map(|detail| format!(": {detail}"))
                    .unwrap_or_default();
                format!("{} {}{}", failure.outcome, failure.step, suffix)
            })
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&format!(
            "  <system-err>{}</system-err>\n",
            xml_escape(&detail)
        ));
    }
    out.push_str("</testsuite>\n");
    out
}

fn phase_label(phase: LifecyclePhase) -> &'static str {
    match phase {
        LifecyclePhase::Setup => "setup",
        LifecyclePhase::Teardown => "teardown",
    }
}

fn status_label(status: LifecycleStatus) -> &'static str {
    match status {
        LifecycleStatus::Passed => "passed",
        LifecycleStatus::Failed => "failed",
        LifecycleStatus::Aborted => "aborted",
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use duhem_judge::InconclusiveCause;
    use duhem_summary::{
        CheckTotals, CleanupFailureSummary, CriterionSummary, LifecycleBlock, LifecycleScopeSegment,
    };

    #[test]
    fn lifecycle_snapshot_handles_unknown_scope_depth() {
        let scope = ["alpha", "beta", "gamma"]
            .into_iter()
            .enumerate()
            .map(|(index, kind)| LifecycleScopeSegment {
                kind: kind.into(),
                id: (index + 1).to_string(),
            })
            .collect();
        let passed = LifecycleBlock {
            phase: LifecyclePhase::Setup,
            scope,
            status: LifecycleStatus::Passed,
            started_at: "t".into(),
            duration_ms: 1,
            steps: vec![],
            failing_step: None,
        };
        let mut failed = passed.clone();
        failed.phase = LifecyclePhase::Teardown;
        failed.status = LifecycleStatus::Failed;
        let summary = RunSummary::new("r", VerdictState::Pass, vec![], PathBuf::from("."))
            .with_lifecycle(vec![passed, failed]);
        let xml = render(&summary);
        assert!(xml.contains("tests=\"1\" failures=\"1\""), "{xml}");
        assert!(
            xml.contains("name=\"alpha:1 / beta:2 / gamma:3 teardown\""),
            "{xml}"
        );
        assert!(
            xml.contains("<system-out>alpha:1 / beta:2 / gamma:3 setup passed</system-out>"),
            "{xml}"
        );
    }

    fn totals(total: u32, passed: u32, failed: u32, inconclusive: u32) -> CheckTotals {
        CheckTotals {
            total,
            passed,
            failed,
            inconclusive,
        }
    }

    #[test]
    fn pass_run_produces_empty_testcase_per_criterion() {
        let s = RunSummary::new(
            "r1",
            VerdictState::Pass,
            vec![
                CriterionSummary {
                    id: "AC-1".into(),
                    verdict: VerdictState::Pass,
                },
                CriterionSummary {
                    id: "AC-2".into(),
                    verdict: VerdictState::Pass,
                },
            ],
            PathBuf::from("."),
        )
        .with_totals(totals(2, 2, 0, 0));
        let xml = render(&s);
        assert!(xml.contains("tests=\"2\""), "{xml}");
        assert!(xml.contains("failures=\"0\""), "{xml}");
        assert!(xml.contains("<testcase name=\"AC-1\"/>"), "{xml}");
        assert!(xml.contains("<testcase name=\"AC-2\"/>"), "{xml}");
    }

    #[test]
    fn fail_criterion_emits_failure_element() {
        let s = RunSummary::new(
            "r",
            VerdictState::Fail,
            vec![CriterionSummary {
                id: "AC-1".into(),
                verdict: VerdictState::Fail,
            }],
            PathBuf::from("."),
        )
        .with_totals(totals(1, 0, 1, 0));
        let xml = render(&s);
        assert!(xml.contains("<failure type=\"fail\"/>"), "{xml}");
    }

    #[test]
    fn inconclusive_criterion_emits_skipped_with_cause_type() {
        let s = RunSummary::new(
            "r",
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
            vec![CriterionSummary {
                id: "AC-1".into(),
                verdict: VerdictState::Inconclusive(InconclusiveCause::Timeout),
            }],
            PathBuf::from("."),
        )
        .with_totals(totals(1, 0, 0, 1));
        let xml = render(&s);
        assert!(xml.contains("<skipped type=\"timeout\"/>"), "{xml}");
    }

    #[test]
    fn xml_special_chars_in_id_are_escaped() {
        let s = RunSummary::new(
            "r",
            VerdictState::Pass,
            vec![CriterionSummary {
                id: "<AC&1>".into(),
                verdict: VerdictState::Pass,
            }],
            PathBuf::from("."),
        )
        .with_totals(totals(1, 1, 0, 0));
        let xml = render(&s);
        assert!(xml.contains("&lt;AC&amp;1&gt;"), "{xml}");
        assert!(!xml.contains("<AC&1>"), "raw should not appear: {xml}");
    }

    #[test]
    fn cleanup_is_system_error_evidence_not_a_test_failure() {
        let s = RunSummary::new("r", VerdictState::Pass, vec![], PathBuf::from(".")).with_cleanup(
            vec![CleanupFailureSummary {
                step: "delete-record".into(),
                outcome: "error".into(),
                detail: Some("synthetic cleanup error".into()),
            }],
        );
        let xml = render(&s);
        assert!(xml.contains("failures=\"0\""), "{xml}");
        assert!(xml.contains("<system-err>"), "{xml}");
        assert!(xml.contains("delete-record"), "{xml}");
    }

    #[test]
    fn suite_attributes_count_testcase_elements_not_checks() {
        // Three checks rolled up into one criterion. JUnit's `tests`
        // attribute must describe the emitted `<testcase>` elements, or a
        // consumer sees a suite promising three tests and finding one.
        let s = RunSummary::new(
            "r",
            VerdictState::Fail,
            vec![CriterionSummary {
                id: "AC-1".into(),
                verdict: VerdictState::Fail,
            }],
            PathBuf::from("."),
        )
        .with_totals(totals(3, 1, 1, 1));

        let xml = render(&s);
        assert!(xml.contains("tests=\"1\""), "{xml}");
        assert_eq!(xml.matches("<testcase").count(), 1, "{xml}");
    }
}
