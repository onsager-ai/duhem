//! CLI-level integration tests for `duhem run` flags (issue #23).
//!
//! `#[ignore]`'d by default: the `duhem run` dispatch unconditionally
//! launches a Playwright browser before invoking the engine (a
//! launch-once-then-reuse policy that predates this spec), so even
//! browser-free fixtures need `npx playwright install chromium` to
//! reach the reporter / evidence code paths these tests exercise.
//! `just test-cli-smoke` (or `cargo test -p duhem-cli -- --ignored`)
//! runs them locally; the matching unit tests in `filter.rs`,
//! `reporter.rs`, and the engine-level test in
//! `duhem-runtime::engine::runner` cover the same behaviour without
//! the browser dependency.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_duhem"))
}

/// Write a stepless Verification Definition to a tempfile. Stepless
/// because we want to drive the CLI without a browser — any step
/// requiring a `Page` would surface as `Inconclusive(EnvironmentError)`.
fn fixture(tmp: &tempfile::TempDir, yaml: &str) -> PathBuf {
    let path = tmp.path().join("v.yml");
    std::fs::write(&path, yaml).expect("write fixture");
    path
}

const ONE_CRITERION: &str = r#"
verification: smoke
criteria:
  - id: AC-1
    description: trivially passing
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;

const TWO_CRITERIA: &str = r#"
verification: smoke
criteria:
  - id: AC-1
    description: passes
    checks:
      - id: AC-1.1
        assertions: ["true"]
      - id: AC-1.2
        assertions: ["false"]
  - id: AC-2
    description: also passes
    checks:
      - id: AC-2.1
        assertions: ["true"]
"#;

#[test]
#[ignore = "requires `npx playwright install chromium`; `duhem run` launches a browser unconditionally"]
fn default_reporter_matches_pre_spec_output_byte_for_byte() {
    // Regression-safety: pre-issue-#23 `duhem run` printed exactly
    // `<verdict>\n`. The `default` reporter must produce the same
    // bytes so existing CI scripts grepping for `pass` keep working.
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    let evidence = tmp.path().join("evidence");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--evidence-dir")
        .arg(&evidence)
        .output()
        .expect("spawn duhem");

    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"pass\n");
}

#[test]
#[ignore = "requires `npx playwright install chromium`; `duhem run` launches a browser unconditionally"]
fn quiet_reporter_writes_nothing_to_stdout() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    let evidence = tmp.path().join("evidence");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--reporter")
        .arg("quiet")
        .arg("--evidence-dir")
        .arg(&evidence)
        .output()
        .expect("spawn duhem");

    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "stdout was {:?}", out.stdout);
}

#[test]
#[ignore = "requires `npx playwright install chromium`; `duhem run` launches a browser unconditionally"]
fn json_reporter_emits_one_valid_json_line() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    let evidence = tmp.path().join("evidence");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--reporter")
        .arg("json")
        .arg("--evidence-dir")
        .arg(&evidence)
        .output()
        .expect("spawn duhem");

    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let trimmed = stdout.trim_end_matches('\n');
    assert!(
        !trimmed.contains('\n'),
        "expected single line, got: {stdout:?}"
    );
    let v: serde_json::Value = serde_json::from_str(trimmed).expect("valid JSON");
    assert_eq!(v["verdict"], "pass");
    assert_eq!(v["criteria"][0]["id"], "AC-1");
    assert!(
        v["evidence_dir"]
            .as_str()
            .unwrap()
            .starts_with(evidence.to_str().unwrap()),
        "evidence_dir should live under --evidence-dir, got {:?}",
        v["evidence_dir"],
    );
}

#[test]
#[ignore = "requires `npx playwright install chromium`; `duhem run` launches a browser unconditionally"]
fn evidence_dir_lands_trace_under_caller_path_and_creates_missing_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    // Intentionally point at a path whose parent doesn't exist yet —
    // the writer must create the chain (spec on #23).
    let evidence = tmp.path().join("nested").join("not-yet").join("runs");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--evidence-dir")
        .arg(&evidence)
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        evidence.is_dir(),
        "evidence dir was not created: {evidence:?}"
    );
    let runs: Vec<_> = std::fs::read_dir(&evidence)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(runs.len(), 1, "exactly one run subdir");
    let trace = runs[0].path().join("trace.jsonl");
    assert!(
        trace.is_file(),
        "trace.jsonl missing under {:?}",
        runs[0].path()
    );
}

#[test]
#[ignore = "requires `npx playwright install chromium`; `duhem run` launches a browser unconditionally"]
fn filter_selects_a_single_check_and_skips_the_failing_sibling() {
    // AC-1.2 fails on its `false` assertion; filtering to AC-1.1 (the
    // passing check) should yield a Pass-at-AC-1, plus AC-2 also
    // running. But AC-2 still has its own check — wait, --filter
    // AC-1::AC-1.1 means AC-2 has no matching checks → AC-2 is empty
    // → run is Inconclusive. To get a clean Pass we'd need to also
    // include AC-2. This test exercises the filter-matters case
    // (failing check skipped) and the all-filtered-criterion case at
    // the same time.
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, TWO_CRITERIA);
    let evidence = tmp.path().join("evidence");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--filter")
        .arg("AC-1::AC-1.1")
        .arg("--reporter")
        .arg("json")
        .arg("--evidence-dir")
        .arg(&evidence)
        .output()
        .expect("spawn duhem");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    // AC-1.1 passes → AC-1 is Pass; AC-2 has no matching checks → empty → Inconclusive.
    // Run aggregates to Inconclusive(EmptyAggregation).
    assert_eq!(v["verdict"], "inconclusive:empty_aggregation");
    let ac1 = v["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "AC-1")
        .unwrap();
    assert_eq!(ac1["verdict"], "pass");
    let ac2 = v["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "AC-2")
        .unwrap();
    assert_eq!(ac2["verdict"], "inconclusive:empty_aggregation");
}

#[test]
#[ignore = "requires `npx playwright install chromium`; `duhem run` launches a browser unconditionally"]
fn filter_with_or_includes_both_criteria_and_run_passes() {
    // Spec on #23: multiple `--filter` flags OR. Picking the passing
    // check from AC-1 and AC-2's only check should land us at Pass.
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, TWO_CRITERIA);
    let evidence = tmp.path().join("evidence");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--filter")
        .arg("AC-1::AC-1.1")
        .arg("--filter")
        .arg("AC-2")
        .arg("--evidence-dir")
        .arg(&evidence)
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"pass\n");
}

#[test]
fn invalid_filter_pattern_errors_before_browser_launch() {
    // Empty-criterion patterns are explicitly rejected (#23). They
    // should fail fast with exit code != 0 and a recognizable
    // message on stderr — without needing a Playwright browser.
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--filter")
        .arg("::AC-1.1")
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("empty criterion"),
        "stderr should name the bad pattern: {stderr}"
    );
}
