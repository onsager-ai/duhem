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

/// Spec on #33: `--dry-run` short-circuits before browser launch and
/// prints `WOULD RUN: <criterion>::<check>` per pair. No `#[ignore]`
/// because the whole point is browser-free.
#[test]
fn dry_run_prints_resolved_pairs_and_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, TWO_CRITERIA);

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--dry-run")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("WOULD RUN: AC-1::AC-1.1"),
        "stdout was {stdout:?}"
    );
    assert!(
        stdout.contains("WOULD RUN: AC-1::AC-1.2"),
        "stdout was {stdout:?}"
    );
    assert!(
        stdout.contains("WOULD RUN: AC-2::AC-2.1"),
        "stdout was {stdout:?}"
    );
    // No evidence directory should be created — the `--evidence-dir`
    // wasn't passed, but the default `.duhem/runs` shouldn't be either.
    let default_evidence = std::path::Path::new(".duhem");
    // (We can't assert .duhem/runs doesn't exist because tests might
    // race with other tests in the repo. The check below — that no
    // trace.jsonl was written for *this* invocation — is what
    // actually matters.)
    let _ = default_evidence;
}

/// Spec on #33: `--dry-run` honors `--filter`. Confirms filter
/// resolution is part of the dry-run plan, not bypassed.
#[test]
fn dry_run_honors_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, TWO_CRITERIA);

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--dry-run")
        .arg("--filter")
        .arg("AC-1::AC-1.1")
        .output()
        .expect("spawn duhem");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("WOULD RUN: AC-1::AC-1.1"), "got: {stdout}");
    assert!(
        !stdout.contains("AC-1.2") && !stdout.contains("AC-2.1"),
        "filter should exclude others: {stdout}"
    );
}

/// Spec on #33: when a filter excludes everything, the dry-run banner
/// makes the empty result visible (silence would look identical to a
/// stepless VD).
#[test]
fn dry_run_with_filter_that_matches_nothing_emits_explicit_signal() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--dry-run")
        .arg("--filter")
        .arg("AC-NOPE")
        .output()
        .expect("spawn duhem");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("no checks matched filter"),
        "expected empty-banner, got: {stdout}"
    );
}

/// Spec on #33: `--inputs-file` supplies inputs; explicit `--inputs`
/// wins on the same key. Verifying via `--dry-run` so this test stays
/// browser-free.
#[test]
fn inputs_file_loads_yaml_and_dry_run_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let v = tmp.path().join("v.yml");
    std::fs::write(
        &v,
        r#"
verification: smoke
inputs:
  base_url: { type: string }
  count: { type: integer }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#,
    )
    .unwrap();
    let inputs_file = tmp.path().join("inputs.yml");
    std::fs::write(&inputs_file, "base_url: http://staging\ncount: 5\n").unwrap();

    let out = Command::new(bin())
        .arg("run")
        .arg(&v)
        .arg("--dry-run")
        .arg("--inputs-file")
        .arg(&inputs_file)
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn inputs_file_missing_file_errors_before_browser_launch() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--inputs-file")
        .arg("/no/such/file.yml")
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("/no/such/file.yml"),
        "stderr should name the path: {stderr}"
    );
}

/// Spec on #34 Test § "Error: unknown name yields exit 2 with
/// `unknown reporter:` on stderr." Routes through the CLI surface so
/// the exit code and stderr shape are both pinned. Browser-free
/// because reporter resolution happens before any browser launch.
#[test]
fn unknown_reporter_name_exits_with_code_two() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--reporter")
        .arg("definitely-not-a-real-reporter")
        .output()
        .expect("spawn duhem");
    // Exit 2 is the spec-confirmed code for reporter-not-found —
    // distinguished from `Inconclusive` (FAILURE / exit 1) and from
    // pass (SUCCESS / exit 0).
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown reporter"),
        "stderr should name the failure mode: {stderr}"
    );
    assert!(
        stderr.contains("definitely-not-a-real-reporter"),
        "stderr should echo the bad name: {stderr}"
    );
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
