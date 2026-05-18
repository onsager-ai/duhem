//! Reference plugin reporter: a terminal ANSI table over the
//! `RunSummary` plugin contract (spec on issue #34).
//!
//! Reads one line of JSON from stdin (the `RunSummary` v1 shape),
//! renders an ANSI-colored 2-column table to stdout (criterion id +
//! verdict), and exits 0. A malformed or unsupported `schema_version`
//! exits 2 with a recognizable message on stderr — plugin-side parse
//! failures must surface as reporter errors, not as a silently-zero
//! exit with empty stdout.
//!
//! This is *not* built into the `duhem` binary. It is shipped as a
//! separate crate so it proves the subprocess contract end-to-end: a
//! customer plugin written in any language follows the same protocol.

use std::io::{self, Read};

use duhem_judge::VerdictState;
use duhem_summary::RunSummary;

fn main() {
    let mut buf = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut buf) {
        eprintln!("duhem-reporter-pretty: read stdin: {e}");
        std::process::exit(2);
    }
    let line = buf.trim().lines().next().unwrap_or("");
    let summary: RunSummary = match serde_json::from_str(line) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("duhem-reporter-pretty: parse RunSummary: {e}");
            std::process::exit(2);
        }
    };

    // Refuse to render shapes we don't recognize. A plugin that
    // silently misrenders a future contract is worse than one that
    // fails loudly — the author wants to know to upgrade.
    if summary.schema_version != RunSummary::SCHEMA_VERSION {
        eprintln!(
            "duhem-reporter-pretty: unsupported RunSummary schema_version `{}` (this plugin understands `{}`)",
            summary.schema_version,
            RunSummary::SCHEMA_VERSION,
        );
        std::process::exit(2);
    }

    render(&summary, &mut io::stdout()).unwrap_or_else(|e| {
        eprintln!("duhem-reporter-pretty: write stdout: {e}");
        std::process::exit(2);
    });
}

fn render(s: &RunSummary, out: &mut dyn io::Write) -> io::Result<()> {
    let title = format!("run {} — {}", s.run_id, verdict_label(&s.verdict));
    writeln!(out, "{title}")?;
    // Column width: long enough for `AC-99.99` style ids; verdict
    // column auto-sizes off the rendered label width. ANSI codes are
    // present but minimal — bold/dim only, no full-color palette — so
    // the output is readable on terminals without truecolor.
    let id_w = s
        .criteria
        .iter()
        .map(|c| c.id.len())
        .max()
        .unwrap_or(0)
        .max("CRITERION".len());
    writeln!(out, "{:id_w$}  VERDICT", "CRITERION", id_w = id_w)?;
    let rule = "-".repeat(id_w + 2 + "VERDICT".len());
    writeln!(out, "{rule}")?;
    for c in &s.criteria {
        writeln!(
            out,
            "{:id_w$}  {}",
            c.id,
            verdict_label(&c.verdict),
            id_w = id_w
        )?;
    }
    writeln!(out, "evidence: {}", s.evidence_dir.display())?;
    Ok(())
}

fn verdict_label(v: &VerdictState) -> String {
    // Same string the CLI built-in `default` reporter uses. Reusing
    // the canonical format keeps a `pretty` plugin output greppable
    // for `pass` / `fail` / `inconclusive:*` the same way as built-in
    // output.
    format!("{v}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use duhem_summary::CriterionSummary;

    #[test]
    fn renders_pass_run_to_a_2_column_table() {
        let s = RunSummary::new(
            "01J000000000000000000RUN",
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
            PathBuf::from(".duhem/runs/01J000000000000000000RUN"),
        );
        let mut buf = Vec::new();
        render(&s, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(
            out.contains("run 01J000000000000000000RUN — pass"),
            "got: {out}"
        );
        assert!(out.contains("CRITERION"), "header present: {out}");
        assert!(out.contains("AC-1"), "AC-1 row: {out}");
        assert!(out.contains("AC-2"), "AC-2 row: {out}");
        assert!(
            out.contains("evidence: .duhem/runs/01J000000000000000000RUN"),
            "evidence dir footer: {out}"
        );
    }

    #[test]
    fn renders_inconclusive_wire_label() {
        use duhem_judge::InconclusiveCause;
        let s = RunSummary::new(
            "r",
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
            vec![CriterionSummary {
                id: "AC-1".into(),
                verdict: VerdictState::Inconclusive(InconclusiveCause::Timeout),
            }],
            PathBuf::from("."),
        );
        let mut buf = Vec::new();
        render(&s, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("inconclusive:timeout"), "got: {out}");
    }
}
