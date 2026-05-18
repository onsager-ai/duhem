//! Reference plugin reporter: JUnit XML over the `RunSummary` plugin
//! contract (spec on issue #34).
//!
//! Mapping (deliberately minimal — JUnit consumers vary on what they
//! accept, so we stick to the subset every consumer parses):
//!
//! - One `<testsuite>` per run.
//! - One `<testcase>` per criterion.
//! - `pass` → empty testcase.
//! - `fail` → `<testcase><failure type="fail"/></testcase>`.
//! - `inconclusive:<cause>` → `<testcase><skipped type="<cause>"/></testcase>`.
//!
//! Exits 0 on success, 2 on parse / schema-version failure (mirrors
//! the `pretty` reference plugin).

use std::io::{self, Read};

use duhem_judge::VerdictState;
use duhem_summary::RunSummary;

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
    print!("{xml}");
}

/// Render the `RunSummary` as a JUnit XML document. Returned as a
/// `String` so the test below can assert on the exact wire shape
/// without going through stdout.
fn render(s: &RunSummary) -> String {
    let total = s.criteria.len();
    let failures = s
        .criteria
        .iter()
        .filter(|c| matches!(c.verdict, VerdictState::Fail))
        .count();
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
    out.push_str("</testsuite>\n");
    out
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
    use duhem_summary::CriterionSummary;

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
        );
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
        );
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
        );
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
        );
        let xml = render(&s);
        assert!(xml.contains("&lt;AC&amp;1&gt;"), "{xml}");
        assert!(!xml.contains("<AC&1>"), "raw should not appear: {xml}");
    }
}
