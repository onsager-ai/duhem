//! Native commands use the existing CLI subprocess runner and evidence masker.
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use duhem_actions::{Action, ActionCtx, Invoke, Outcome};
use duhem_evidence::{EventPayload, EvidenceWriter};
use duhem_judge::{InconclusiveCause, VerdictState};
use duhem_schema::{NativeOutputExpect, NativeRun};
use duhem_summary::NativeRunSummary;

use super::EngineError;

/// Native suites (cargo test, pytest, Playwright) run for minutes, so the
/// 5-second `cli/invoke` default does not apply; this only guards hangs.
const DEFAULT_NATIVE_TIMEOUT_MS: u64 = 30 * 60 * 1000;
/// Failure detail keeps the end of each stream; the full output is in the blobs.
const DETAIL_TAIL_LINES: usize = 20;

pub(crate) async fn run_native(
    writer: &mut EvidenceWriter,
    runs: &[NativeRun],
    dir: Option<&Path>,
) -> Result<Vec<NativeRunSummary>, EngineError> {
    let mut results = Vec::new();
    for run in runs {
        let started = Instant::now();
        let cwd = dir
            .unwrap_or_else(|| Path::new("."))
            .join(run.cwd.as_deref().unwrap_or("."));
        let mut with = serde_json::json!({"command": run.command, "cwd": cwd, "env": run.env});
        with["timeout"] = match &run.timeout {
            Some(timeout) => serde_json::to_value(timeout).expect("duration serializes"),
            None => serde_json::json!(DEFAULT_NATIVE_TIMEOUT_MS),
        };
        let with = serde_yml::to_value(with).expect("native command serializes");
        let boundaries = BTreeMap::new();
        let ctx = ActionCtx {
            page: None,
            step_index: 0,
            step_boundaries: &boundaries,
            session: None,
        };
        let executed = Invoke.invoke(&ctx, &with).await;
        let mut result = NativeRunSummary {
            id: writer.mask_text(&run.id),
            layer: run.layer.map(|layer| {
                serde_json::to_value(layer)
                    .expect("layer serializes")
                    .as_str()
                    .unwrap()
                    .to_string()
            }),
            outcome: VerdictState::Pass,
            duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            required: run.required,
            tests: 1,
            exit_code: None,
            detail: None,
            stdout_blob_sha256: None,
            stderr_blob_sha256: None,
        };
        match executed {
            Err(error) => {
                result.outcome = VerdictState::Inconclusive(InconclusiveCause::EnvironmentError);
                result.detail = Some(writer.mask_text(&error.to_string()));
            }
            Ok(output) if output.outcome == Outcome::Timeout => {
                result.outcome = VerdictState::Inconclusive(InconclusiveCause::Timeout);
                result.detail = Some("native command timed out".into());
            }
            Ok(output) => {
                let stdout = output
                    .outputs
                    .get("stdout")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let stderr = output
                    .outputs
                    .get("stderr")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                result.stdout_blob_sha256 = Some(writer.write_blob(stdout.as_bytes()).await?.0);
                result.stderr_blob_sha256 = Some(writer.write_blob(stderr.as_bytes()).await?.0);
                result.exit_code = output
                    .outputs
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .map(|code| code as i32);
                let expected = run.expect.as_ref().map_or(0, |expect| expect.exit_code);
                let mut failures = Vec::new();
                if result.exit_code != Some(expected) {
                    failures.push(format!(
                        "expected exit code {expected}, observed {:?}",
                        result.exit_code
                    ));
                }
                if let Some(expect) = &run.expect {
                    check_output("stdout", stdout, expect.stdout.as_ref(), &mut failures);
                    check_output("stderr", stderr, expect.stderr.as_ref(), &mut failures);
                }
                if !failures.is_empty() {
                    result.outcome = VerdictState::Fail;
                    result.detail = Some(writer.mask_text(&format!(
                        "{}\nstdout: {}\nstderr: {}",
                        failures.join("; "),
                        tail(stdout),
                        tail(stderr)
                    )));
                }
            }
        }
        writer
            .append(EventPayload::NativeRunFinished {
                result: result.clone(),
            })
            .await?;
        results.push(result);
    }
    Ok(results)
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    match lines.len().checked_sub(DETAIL_TAIL_LINES) {
        Some(skipped) if skipped > 0 => format!(
            "[{skipped} earlier lines in the output blob]\n{}",
            lines[skipped..].join("\n")
        ),
        _ => lines.join("\n"),
    }
}

fn check_output(
    stream: &str,
    text: &str,
    expect: Option<&NativeOutputExpect>,
    failures: &mut Vec<String>,
) {
    let Some(expect) = expect else {
        return;
    };
    for (pattern, should_match) in [(&expect.matches, true), (&expect.not_matches, false)] {
        if let Some(pattern) = pattern {
            match regex::Regex::new(pattern) {
                Ok(regex) if regex.is_match(text) == should_match => {}
                Ok(_) => failures.push(format!(
                    "{stream} {} `{pattern}`",
                    if should_match {
                        "does not match"
                    } else {
                        "matches forbidden pattern"
                    }
                )),
                Err(error) => failures.push(format!("invalid {stream} regex: {error}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn detail_keeps_only_the_output_tail() {
        let long: String = (1..=25).map(|n| format!("line {n}\n")).collect();
        let tail = super::tail(&long);
        assert!(tail.starts_with("[5 earlier lines in the output blob]\nline 6\n"));
        assert!(tail.ends_with("line 25"));
        assert_eq!(super::tail("a\nb\n"), "a\nb");
    }
}
