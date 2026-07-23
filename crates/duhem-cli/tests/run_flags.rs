//! CLI-level integration tests for `duhem run` flags (issue #23).
//!
//! `#[ignore]`'d by default: the `duhem run` dispatch unconditionally
//! launches a Playwright browser before invoking the engine (a
//! launch-once-then-reuse policy that predates this spec), so even
//! browser-free fixtures need `npx playwright install chromium` to
//! reach the reporter / store code paths these tests exercise.
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
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--db")
        .arg(&db)
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
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--reporter")
        .arg("quiet")
        .arg("--db")
        .arg(&db)
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
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--reporter")
        .arg("json")
        .arg("--db")
        .arg(&db)
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
    assert_eq!(
        v["store"].as_str().unwrap(),
        db.to_str().unwrap(),
        "store should be the --db path, got {:?}",
        v["store"],
    );
}

#[test]
#[ignore = "requires `npx playwright install chromium`; `duhem run` launches a browser unconditionally"]
fn db_flag_lands_store_at_caller_path_and_creates_missing_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    // Intentionally point at a path whose parent doesn't exist yet —
    // the store open must create the chain (specs #23 / #189).
    let db = tmp.path().join("nested").join("not-yet").join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--db")
        .arg(&db)
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(db.is_file(), "store DB was not created: {db:?}");
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
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--filter")
        .arg("AC-1::AC-1.1")
        .arg("--reporter")
        .arg("json")
        .arg("--db")
        .arg(&db)
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
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--filter")
        .arg("AC-1::AC-1.1")
        .arg("--filter")
        .arg("AC-2")
        .arg("--db")
        .arg(&db)
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
    // No store should be touched — `--dry-run` returns before the
    // store is opened (spec #189: a dry run writes nothing).
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

/// A VD with one required integer input and one optional string input.
/// Used by the #151 `--inputs @file` tests: the required `count` makes
/// "did the file load?" observable (a missing value fails resolution),
/// and its `integer` type is the lever for asserting last-wins (the
/// losing string `count=notanumber` would fail coercion).
const COUNT_VD: &str = r#"
verification: smoke
inputs:
  count: { type: integer }
  name: { type: string, default: anon }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;

/// Spec on #151: `--inputs @file` loads a YAML/JSON mapping; verifying
/// via `--dry-run` so this test stays browser-free. The control case
/// (no `--inputs`) fails resolution on the required `count`, proving the
/// `@file` is what supplied it.
#[test]
fn inputs_at_file_loads_yaml_and_dry_run_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let v = fixture(&tmp, COUNT_VD);
    let inputs_file = tmp.path().join("inputs.yml");
    std::fs::write(&inputs_file, "count: 5\n").unwrap();

    let out = Command::new(bin())
        .arg("run")
        .arg(&v)
        .arg("--dry-run")
        .arg("--inputs")
        .arg(format!("@{}", inputs_file.display()))
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Control: without the `@file`, the required `count` is unresolved.
    let out = Command::new(bin())
        .arg("run")
        .arg(&v)
        .arg("--dry-run")
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success(), "missing required input should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("missing required input") && stderr.contains("count"),
        "stderr should name the missing input: {stderr}"
    );
}

/// Spec on #151: `--inputs` tokens are last-wins in declared order. We
/// observe which value is in effect via the `integer` type lever — the
/// losing token never gets coerced, so order flips success/failure.
#[test]
fn inputs_last_token_wins_across_mixed_tokens() {
    let tmp = tempfile::tempdir().unwrap();
    let v = fixture(&tmp, COUNT_VD);
    let good = tmp.path().join("count.yml");
    std::fs::write(&good, "count: 7\n").unwrap();
    let good_tok = format!("@{}", good.display());

    // `@file` then a non-integer flag → the flag wins → coercion fails.
    let out = Command::new(bin())
        .arg("run")
        .arg(&v)
        .arg("--dry-run")
        .arg("--inputs")
        .arg(&good_tok)
        .arg("--inputs")
        .arg("count=notanumber")
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success(), "raw token should win and fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("integer"), "stderr: {stderr}");

    // Reversed order → the valid `@file` value wins → success.
    let out = Command::new(bin())
        .arg("run")
        .arg(&v)
        .arg("--dry-run")
        .arg("--inputs")
        .arg("count=notanumber")
        .arg("--inputs")
        .arg(&good_tok)
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "file token should win; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Spec on #155: `--dry-run` prints a `RESOLVED INPUT: <name> = <value>`
/// line per input with the *post-precedence* value (`--inputs` last-wins,
/// then env, then default). This is the value-level assertion that was
/// only reachable indirectly before (via a type-coercion lever): the
/// winning value is now visible directly on stdout.
#[test]
fn dry_run_prints_resolved_input_values_post_precedence() {
    let tmp = tempfile::tempdir().unwrap();
    let v = fixture(&tmp, COUNT_VD);
    let file = tmp.path().join("count.yml");
    std::fs::write(&file, "count: 7\n").unwrap();
    let file_tok = format!("@{}", file.display());

    // `@file(count=7)` then `count=42` → the flag is last, so 42 wins.
    let out = Command::new(bin())
        .arg("run")
        .arg(&v)
        .arg("--dry-run")
        .arg("--inputs")
        .arg(&file_tok)
        .arg("--inputs")
        .arg("count=42")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Integer rendered as the coerced value (no quotes); `name` falls
    // back to its `default: anon`.
    assert!(
        stdout.contains("RESOLVED INPUT: count = 42"),
        "stdout was {stdout:?}"
    );
    assert!(
        stdout.contains("RESOLVED INPUT: name = anon"),
        "default should resolve; stdout was {stdout:?}"
    );
    // The losing token's value is not the resolved one.
    assert!(
        !stdout.contains("RESOLVED INPUT: count = 7"),
        "losing value leaked: {stdout:?}"
    );

    // Reversed order → the `@file` value (7) is last, so 7 wins.
    let out = Command::new(bin())
        .arg("run")
        .arg(&v)
        .arg("--dry-run")
        .arg("--inputs")
        .arg("count=42")
        .arg("--inputs")
        .arg(&file_tok)
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("RESOLVED INPUT: count = 7"),
        "file token should win; stdout was {stdout:?}"
    );
}

/// Spec on #155: a `string` input renders bare (no surrounding quotes)
/// so a VD substring-asserts the value cleanly, and a `--inputs` flag
/// beats the declared `default:`.
#[test]
fn dry_run_resolved_input_renders_string_bare_over_default() {
    let tmp = tempfile::tempdir().unwrap();
    let v = fixture(&tmp, COUNT_VD);

    let out = Command::new(bin())
        .arg("run")
        .arg(&v)
        .arg("--dry-run")
        .arg("--inputs")
        .arg("count=1")
        .arg("--inputs")
        .arg("name=hello world")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Bare string (no quotes); the flag beat the `anon` default.
    assert!(
        stdout.contains("RESOLVED INPUT: name = hello world"),
        "stdout was {stdout:?}"
    );
    assert!(
        !stdout.contains("name = anon"),
        "the flag should override the default: {stdout:?}"
    );
}

/// Spec on #151: `key=@literal` keeps `@literal` as a literal value; the
/// `@` only triggers file-loading as a bare leading token. We prove the
/// distinction by contrast: the same `@nope.yml` string is a literal
/// after `=` (succeeds) but a missing-file ref as a bare token (fails).
#[test]
fn key_at_literal_is_not_treated_as_file() {
    let tmp = tempfile::tempdir().unwrap();
    let v = fixture(&tmp, COUNT_VD);
    let good = tmp.path().join("count.yml");
    std::fs::write(&good, "count: 7\n").unwrap();
    let good_tok = format!("@{}", good.display());

    // `name=@nope.yml` → literal string value, no file load → success.
    let out = Command::new(bin())
        .arg("run")
        .arg(&v)
        .arg("--dry-run")
        .arg("--inputs")
        .arg(&good_tok)
        .arg("--inputs")
        .arg("name=@nope.yml")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "key=@literal must not load a file; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The same string as a bare leading token IS a file ref → missing.
    let out = Command::new(bin())
        .arg("run")
        .arg(&v)
        .arg("--dry-run")
        .arg("--inputs")
        .arg(&good_tok)
        .arg("--inputs")
        .arg("@nope.yml")
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success(), "bare @file should load and fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("nope.yml"),
        "stderr should name file: {stderr}"
    );
}

#[test]
fn inputs_missing_at_file_errors_before_browser_launch() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--inputs")
        .arg("@/no/such/file.yml")
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("/no/such/file.yml"),
        "stderr should name the path: {stderr}"
    );
}

/// Spec on #151: the pruned flags `--seed`, `--headed`, `--inputs-file`
/// are now unknown to clap — each fails fast with a non-zero exit and an
/// "unexpected argument" diagnostic, before any browser launch.
#[test]
fn removed_flags_are_rejected_as_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);

    for args in [
        vec!["--seed", "1"],
        vec!["--headed"],
        vec!["--inputs-file", "x.yml"],
    ] {
        let out = Command::new(bin())
            .arg("run")
            .arg(&path)
            .args(&args)
            .output()
            .expect("spawn duhem");
        assert!(
            !out.status.success(),
            "`{args:?}` should be rejected as unknown"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("unexpected argument") || stderr.contains(args[0]),
            "stderr should flag the unknown arg `{}`: {stderr}",
            args[0]
        );
    }
}

/// Spec on #34 Test § "Error: unknown name yields exit 2 with
/// `unknown reporter:` on stderr." Routes through the CLI surface so
/// the exit code and stderr shape are both pinned. Browser-free
/// because reporter resolution happens before any browser launch.
#[test]
fn unknown_reporter_name_exits_with_code_two() {
    // Isolate `HOME` and the current directory under a tempdir so the
    // CLI's reporter resolver never reads the developer's real
    // `~/.duhem/config.toml` or an ancestor's `.duhem.toml`. Without
    // this, a stale or malformed local config would surface here as a
    // load-time failure instead of the expected unknown-reporter
    // path (Copilot review on PR #43).
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--reporter")
        .arg("definitely-not-a-real-reporter")
        .current_dir(tmp.path())
        .env("HOME", tmp.path())
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

// ---- #49: root manifest loader / multi-leaf `duhem run` -------------

/// Lay down a two-leaf manifest tree under `dir/`. Returns the
/// manifest path. Used by every #49 integration test.
fn manifest_with_two_leaves(dir: &std::path::Path) -> PathBuf {
    let leaf_a = dir.join("leaf-a");
    let leaf_b = dir.join("leaf-b");
    std::fs::create_dir_all(&leaf_a).unwrap();
    std::fs::create_dir_all(&leaf_b).unwrap();
    std::fs::write(
        leaf_a.join("duhem.yml"),
        r#"
verification: leaf-a
criteria:
  - id: AC-1
    description: passes
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#,
    )
    .unwrap();
    std::fs::write(
        leaf_b.join("duhem.yml"),
        r#"
verification: leaf-b
criteria:
  - id: AC-1
    description: passes
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#,
    )
    .unwrap();
    let manifest = dir.join("duhem.yml");
    std::fs::write(
        &manifest,
        r#"
manifest_version: 1
verifications:
  - path: ./leaf-a/duhem.yml
  - path: ./leaf-b/duhem.yml
"#,
    )
    .unwrap();
    manifest
}

#[test]
fn dry_run_on_manifest_qualifies_pairs_with_verification_name() {
    // Spec on #49 Test: a root manifest with two leaves dry-runs both
    // leaves and qualifies each WOULD RUN line with the leaf name.
    let tmp = tempfile::tempdir().unwrap();
    let manifest = manifest_with_two_leaves(tmp.path());

    let out = Command::new(bin())
        .arg("run")
        .arg(&manifest)
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
        stdout.contains("WOULD RUN: leaf-a::AC-1::AC-1.1"),
        "stdout was {stdout:?}"
    );
    assert!(
        stdout.contains("WOULD RUN: leaf-b::AC-1::AC-1.1"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn dry_run_resolved_inputs_qualified_by_verification_name_on_manifest() {
    // Spec on #155: on a manifest run the `RESOLVED INPUT` lines are
    // qualified with the leaf name, mirroring the `WOULD RUN` lines. Both
    // leaves declare no inputs, so each emits a qualified `(none)` line —
    // exercising both the qualification and the empty-input rendering.
    let tmp = tempfile::tempdir().unwrap();
    let manifest = manifest_with_two_leaves(tmp.path());

    let out = Command::new(bin())
        .arg("run")
        .arg(&manifest)
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
        stdout.contains("RESOLVED INPUT: leaf-a:: (none)"),
        "stdout was {stdout:?}"
    );
    assert!(
        stdout.contains("RESOLVED INPUT: leaf-b:: (none)"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn dry_run_on_directory_path_resolves_to_duhem_yml() {
    // Spec on #49 § "CLI surface": directory paths resolve to
    // `<dir>/duhem.yml`. Same dry-run output as passing the manifest
    // file directly.
    let tmp = tempfile::tempdir().unwrap();
    let _manifest = manifest_with_two_leaves(tmp.path());

    let out = Command::new(bin())
        .arg("run")
        .arg(tmp.path())
        .arg("--dry-run")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("leaf-a::AC-1::AC-1.1"), "got {stdout:?}");
    assert!(stdout.contains("leaf-b::AC-1::AC-1.1"), "got {stdout:?}");
}

#[test]
fn dry_run_three_part_filter_selects_within_named_leaf_only() {
    // Spec on #49: `--filter foo::AC-1::AC-1.1` selects across the
    // right leaf; other leaves drop out entirely.
    let tmp = tempfile::tempdir().unwrap();
    let manifest = manifest_with_two_leaves(tmp.path());

    let out = Command::new(bin())
        .arg("run")
        .arg(&manifest)
        .arg("--dry-run")
        .arg("--filter")
        .arg("leaf-b::AC-1::AC-1.1")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("leaf-b::AC-1::AC-1.1"),
        "expected leaf-b match: {stdout:?}"
    );
    assert!(
        !stdout.contains("leaf-a::"),
        "leaf-a should be skipped: {stdout:?}"
    );
}

#[test]
fn dry_run_two_part_filter_applies_to_every_leaf() {
    // Spec on #49: `--filter AC-1::AC-1.1` selects in every leaf.
    let tmp = tempfile::tempdir().unwrap();
    let manifest = manifest_with_two_leaves(tmp.path());

    let out = Command::new(bin())
        .arg("run")
        .arg(&manifest)
        .arg("--dry-run")
        .arg("--filter")
        .arg("AC-1::AC-1.1")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("leaf-a::AC-1::AC-1.1"),
        "leaf-a should match: {stdout:?}"
    );
    assert!(
        stdout.contains("leaf-b::AC-1::AC-1.1"),
        "leaf-b should match: {stdout:?}"
    );
}

#[test]
fn directory_without_manifest_errors_before_browser_launch() {
    // Spec on #49 § "CLI surface": directory with no `duhem.yml` is a
    // load-time error.
    let tmp = tempfile::tempdir().unwrap();

    let out = Command::new(bin())
        .arg("run")
        .arg(tmp.path())
        .arg("--dry-run")
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no `duhem.yml`") || stderr.contains("missing manifest"),
        "stderr should name the failure: {stderr}"
    );
}

#[test]
#[ignore = "requires `npx playwright install chromium`; `duhem run` launches a browser per leaf"]
fn manifest_runs_every_leaf_and_aggregates_verdicts() {
    // Spec on #49 Test (integration): a manifest with two leaves runs
    // both, produces two evidence dirs, aggregates correctly; CLI
    // exit code reflects the aggregated verdict.
    let tmp = tempfile::tempdir().unwrap();
    let manifest = manifest_with_two_leaves(tmp.path());
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&manifest)
        .arg("--db")
        .arg(&db)
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Both leaves passed → run-set verdict is `pass`. Default
    // reporter prints one `<name>: <verdict>` line per leaf, then the
    // aggregated verdict on its own line.
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("leaf-a: pass"),
        "expected per-leaf line: {stdout:?}"
    );
    assert!(
        stdout.contains("leaf-b: pass"),
        "expected per-leaf line: {stdout:?}"
    );
    assert!(
        stdout.lines().last().unwrap_or("").trim() == "pass",
        "expected aggregated verdict on last line: {stdout:?}"
    );
    // Both leaf runs landed in the one store.
    assert!(db.is_file(), "store DB missing: {db:?}");
}

// ---- #69: manifest discovery (ancestor walk, `-f` override) --------

#[test]
fn discovery_from_subdir_finds_repo_root_manifest() {
    // Spec on #69 Test (integration): `duhem run` from a sub-directory
    // of a Pattern-B repo discovers the repo-root manifest by walking
    // ancestors — no path argument. Browser-free via `--dry-run`.
    let tmp = tempfile::tempdir().unwrap();
    let _manifest = manifest_with_two_leaves(tmp.path());
    // A `.git` at the repo root marks the boundary the walk caps at;
    // it also keeps the walk from escaping the tempdir into the host.
    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    // A manifest-less nested directory: the walk has to climb past it
    // to the repo-root `duhem.yml`.
    let subdir = tmp.path().join("nested").join("deep");
    std::fs::create_dir_all(&subdir).unwrap();

    let out = Command::new(bin())
        .arg("run")
        .arg("--dry-run")
        .current_dir(&subdir)
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Resolved the root manifest → both leaves planned.
    assert!(
        stdout.contains("leaf-a::AC-1::AC-1.1"),
        "stdout was {stdout:?}"
    );
    assert!(
        stdout.contains("leaf-b::AC-1::AC-1.1"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn file_override_resolves_out_of_tree_manifest() {
    // Spec on #69: `-f <path>` is the explicit override — used as-is,
    // bypassing discovery. Run from an unrelated cwd to prove the flag
    // (not the cwd) drives resolution. Browser-free via `--dry-run`.
    let tmp = tempfile::tempdir().unwrap();
    let manifest = manifest_with_two_leaves(tmp.path());
    let elsewhere = tempfile::tempdir().unwrap();

    let out = Command::new(bin())
        .arg("run")
        .arg("-f")
        .arg(&manifest)
        .arg("--dry-run")
        .current_dir(elsewhere.path())
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("leaf-a::AC-1::AC-1.1"), "got {stdout:?}");
}

#[test]
fn discovery_with_no_manifest_anywhere_errors() {
    // Spec on #69: exhausting the walk (capped at `.git`) without a
    // manifest surfaces a clear "no manifest found" error.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    let subdir = tmp.path().join("sub");
    std::fs::create_dir_all(&subdir).unwrap();

    let out = Command::new(bin())
        .arg("run")
        .arg("--dry-run")
        .current_dir(&subdir)
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no manifest found"),
        "stderr should name the failure: {stderr}"
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

// ---- #191: target identity (project: + resolution ladder) ----------

/// Open the store read-only and return the single run's
/// (project_id, target_repo, target_sha, verifier_repo).
fn scope_of_single_run(
    db: &std::path::Path,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        use duhem_evidence::Store;
        let store = duhem_evidence::SqliteStore::open_read_only(db)
            .await
            .unwrap();
        let runs = store.list_runs().await.unwrap();
        assert_eq!(runs.len(), 1, "one run in the store");
        let s = &runs[0].scope;
        (
            s.project_id.clone(),
            s.target_repo.clone(),
            s.target_sha.clone(),
            s.verifier_repo.clone(),
        )
    })
}

const DECLARED_PROJECT: &str = r#"
verification: idcheck
project:
  repo: github.com/crawlab-team/crawlab-pro
criteria:
  - id: AC-1
    description: trivially passing
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;

/// Spec #191 Test: a declared `project:` populates `project_id` /
/// `target_repo`, beating any CI context. Stepless VD → browser-free.
#[test]
fn declared_project_beats_ci_context_and_lands_in_the_store() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, DECLARED_PROJECT);
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--db")
        .arg(&db)
        // A CI-shaped environment the declaration must beat for
        // identity — while its sha still dates the run.
        .env("GITHUB_REPOSITORY", "acme/from-ci")
        .env("GITHUB_SHA", "cisha123")
        .env_remove("DUHEM_TARGET_REPO")
        .env_remove("DUHEM_TARGET_SHA")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let (project_id, target_repo, target_sha, _) = scope_of_single_run(&db);
    assert_eq!(
        project_id.as_deref(),
        Some("github.com/crawlab-team/crawlab-pro")
    );
    assert_eq!(
        target_repo.as_deref(),
        Some("github.com/crawlab-team/crawlab-pro")
    );
    assert_eq!(target_sha.as_deref(), Some("cisha123"));
}

/// Spec #191 Test: with no declaration, the CI context rung resolves
/// the target (`DUHEM_TARGET_*` beating `GITHUB_*`), and the verifier
/// env override lands in provenance.
#[test]
fn ci_context_resolves_target_when_undeclared() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--db")
        .arg(&db)
        .env("GITHUB_REPOSITORY", "acme/from-gh")
        .env("GITHUB_SHA", "ghsha")
        .env("DUHEM_TARGET_REPO", "gitlab.com/acme/real")
        .env("DUHEM_TARGET_SHA", "realsha")
        .env("DUHEM_VERIFIER_REPO", "github.com/onsager-ai/duhem")
        .env("DUHEM_VERIFIER_SHA", "v0.1.0")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let (project_id, target_repo, target_sha, verifier_repo) = scope_of_single_run(&db);
    assert_eq!(project_id.as_deref(), Some("gitlab.com/acme/real"));
    assert_eq!(target_repo.as_deref(), Some("gitlab.com/acme/real"));
    assert_eq!(target_sha.as_deref(), Some("realsha"));
    assert_eq!(
        verifier_repo.as_deref(),
        Some("github.com/onsager-ai/duhem")
    );
}

/// Spec #191 Test (back-compat): a VD with no `project:` and no CI
/// env still runs; identity falls through to the remote/path rungs
/// without erroring.
#[test]
fn undeclared_project_without_ci_still_runs_and_attributes_something() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--db")
        .arg(&db)
        .env_remove("GITHUB_REPOSITORY")
        .env_remove("GITHUB_SHA")
        .env_remove("DUHEM_TARGET_REPO")
        .env_remove("DUHEM_TARGET_SHA")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Remote rung (running inside the duhem checkout) or the path
    // fallback — either way the hint is populated, never empty.
    let (project_id, _, _, _) = scope_of_single_run(&db);
    assert!(
        project_id.is_some_and(|p| !p.is_empty()),
        "identity hint must be populated"
    );
}

/// Spec #191 Test: `validate` rejects a malformed `project:` block.
#[test]
fn validate_rejects_a_malformed_project_block() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(
        &tmp,
        r#"
verification: bad
project:
  repo: a/b
  id: also-this
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#,
    );
    let out = Command::new(bin())
        .arg("validate")
        .arg(&path)
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("2 coordinates"),
        "stderr should name the rule: {stderr}"
    );
}

/// #298: with a dashboard base configured, `duhem run` prints the live
/// deep link on STDERR before the run — and stdout stays byte-stable
/// (the default reporter's `pass\n` contract above). Not `#[ignore]`d:
/// the fixture is page-free, and page-free runs skip the browser
/// launch entirely (`needs_browser` in `run_cmd.rs`).
#[test]
fn live_link_prints_on_stderr_when_dashboard_base_is_set() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--db")
        .arg(&db)
        .env("DUHEM_DASHBOARD_URL", "http://127.0.0.1:7878")
        .output()
        .expect("spawn duhem");

    assert!(out.status.success());
    assert_eq!(out.stdout, b"pass\n", "stdout stays machine-stable");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("live: http://127.0.0.1:7878/#/run/"),
        "stderr should carry the live deep link: {stderr}"
    );
}

/// #298 flip side: no dashboard base resolvable → no live line, no
/// stderr noise at all.
#[test]
fn no_dashboard_base_means_no_live_line() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--db")
        .arg(&db)
        .env_remove("DUHEM_DASHBOARD_URL")
        .output()
        .expect("spawn duhem");

    assert!(out.status.success());
    assert_eq!(out.stdout, b"pass\n");
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("live:"),
        "no base → no live line"
    );
}

/// #299: `--live` forces per-criterion progress onto stderr even
/// without a TTY (the flag exists exactly for captured-stderr
/// consumers like the self-verification VD), while stdout keeps the
/// default reporter's byte-stable `pass\n`.
#[test]
fn live_flag_renders_per_criterion_progress_on_stderr() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--db")
        .arg(&db)
        .arg("--live")
        .output()
        .expect("spawn duhem");

    assert!(out.status.success());
    assert_eq!(out.stdout, b"pass\n", "stdout stays machine-stable");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("▶ AC-1 (1/1)…"),
        "criterion start line expected: {stderr}"
    );
    assert!(
        stderr.contains("✔ AC-1 pass"),
        "criterion verdict line expected: {stderr}"
    );
}

/// #299 default posture: stderr is not a TTY under `Command`, so
/// without `--live` no progress renders — piped/CI output is
/// byte-identical to the pre-#299 CLI.
#[test]
fn no_tty_and_no_flag_means_no_progress_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture(&tmp, ONE_CRITERION);
    let db = tmp.path().join("duhem.db");

    let out = Command::new(bin())
        .arg("run")
        .arg(&path)
        .arg("--db")
        .arg(&db)
        .output()
        .expect("spawn duhem");

    assert!(out.status.success());
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("▶"),
        "auto mode must stay silent without a TTY"
    );
}

/// #299: `--live --no-live` is a clap conflict, rejected before any
/// work happens.
#[test]
fn live_and_no_live_conflict() {
    let out = Command::new(bin())
        .arg("run")
        .arg("whatever.yml")
        .arg("--live")
        .arg("--no-live")
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot be used with"),
        "clap conflict expected"
    );
}
