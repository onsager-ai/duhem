//! Best-effort post-upload links and GitHub Actions job summaries (#673).
use std::io::Write;

use duhem_judge::VerdictState;
use duhem_runtime::RunOutcome;
use duhem_summary::{CheckTotals, NativeRunSummary, ShardIdentity};

pub fn report(
    delivery: &crate::export_cmd::AutoSend,
    run_id: &str,
    name: &str,
    verdict: VerdictState,
    leaves: &[(String, RunOutcome)],
    suite_native: &[NativeRunSummary],
    shard: Option<&ShardIdentity>,
) {
    let Some(base) = &delivery.base else {
        return;
    };
    let url = crate::live_link::run_page_url(base, run_id);
    let outcome = match &delivery.result {
        Ok(()) => "sent".to_string(),
        Err(error) => format!("upload failed: {error}"),
    };
    eprintln!("server: {url} ({outcome})");

    let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") else {
        return;
    };
    let checks = CheckTotals::from_verdicts(leaves.iter().flat_map(|(_, run)| {
        run.verdict
            .criteria
            .iter()
            .flat_map(|criterion| criterion.checks.iter().map(|check| &check.state))
    }));
    let native: Vec<_> = suite_native
        .iter()
        .chain(leaves.iter().flat_map(|(_, run)| &run.native))
        .collect();
    let mut block = format!(
        "\n### Duhem: {}\n\nVerdict: {verdict}\n\nChecks: {} total, {} passed, {} failed, {} inconclusive\n",
        markdown_text(name),
        checks.total,
        checks.passed,
        checks.failed,
        checks.inconclusive,
    );
    if !native.is_empty() {
        let gates = CheckTotals::from_verdicts(native.iter().map(|run| &run.outcome));
        block.push_str(&format!(
            "\nNative runs: {} total, {} passed, {} failed, {} inconclusive\n",
            gates.total, gates.passed, gates.failed, gates.inconclusive,
        ));
        if native.iter().any(|run| run.counts.is_some()) {
            let mut counts = duhem_summary::NativeTestCounts::default();
            for run in &native {
                if let Some(tests) = &run.counts {
                    counts.passed += tests.passed;
                    counts.failed += tests.failed;
                    counts.errored += tests.errored;
                    counts.skipped += tests.skipped;
                }
            }
            block.push_str(&format!(
                "\nNative tests: {} passed, {} failed, {} errored, {} skipped\n",
                counts.passed, counts.failed, counts.errored, counts.skipped,
            ));
        }
    }
    if let Some(shard) = shard {
        block.push_str(&format!("\nShard: {}/{}\n", shard.index, shard.total));
    }
    block.push_str(&format!(
        "\n[View run](<{url}>) ({})\n",
        markdown_text(&outcome)
    ));
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(block.as_bytes()));
    if let Err(error) = result {
        eprintln!("warning: could not append GitHub job summary: {error}");
    }
}

fn markdown_text(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        match ch {
            '\r' | '\n' => escaped.push(' '),
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}
