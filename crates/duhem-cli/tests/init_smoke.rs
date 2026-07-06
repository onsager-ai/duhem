//! End-to-end smoke for `duhem init` + `duhem validate` + `duhem
//! run`. Spec on issue #48 § Plan: "drive `init` into a tempdir,
//! then `validate`, then `run`; expect `RunVerdict::Pass`. Gate
//! behind the same outbound-network flag the api/* integration
//! tests use."
//!
//! Today that gate is `#[ignore]` plus a clear message — the
//! existing convention in `crates/duhem-cli/tests/run_flags.rs`
//! and `crates/duhem-actions/tests/ui_smoke.rs`. Run locally with
//! `cargo test -p duhem-cli -- --ignored`.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_duhem"))
}

/// Validate-only smoke: needs no browser or network. Confirms the
/// scaffolded YAML round-trips through `duhem validate` from the
/// shipped binary.
#[test]
fn init_then_validate_passes() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("v");

    let init = Command::new(bin())
        .args(["init"])
        .arg(&target)
        .args(["--name", "smoke"])
        .output()
        .expect("spawn init");
    assert!(
        init.status.success(),
        "init stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let validate = Command::new(bin())
        .args(["validate"])
        .arg(target.join("duhem.yml"))
        .output()
        .expect("spawn validate");
    assert!(
        validate.status.success(),
        "validate stderr: {}",
        String::from_utf8_lossy(&validate.stderr)
    );
    assert_eq!(validate.stdout, b"OK\n");
}

/// Full `init → validate → run` against `https://example.com`.
/// Ignored by default — `duhem run` launches Playwright (needs
/// `npx playwright install chromium`) and the skeleton hits a
/// real public URL (needs outbound HTTPS).
#[test]
#[ignore = "requires `npx playwright install chromium` and outbound HTTPS to https://example.com"]
fn init_then_validate_then_run_passes_against_example_com() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("v");
    let db = tmp.path().join("duhem.db");

    let init = Command::new(bin())
        .args(["init"])
        .arg(&target)
        .args(["--name", "smoke"])
        .output()
        .expect("spawn init");
    assert!(
        init.status.success(),
        "init stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let validate = Command::new(bin())
        .args(["validate"])
        .arg(target.join("duhem.yml"))
        .output()
        .expect("spawn validate");
    assert!(
        validate.status.success(),
        "validate stderr: {}",
        String::from_utf8_lossy(&validate.stderr)
    );

    let run = Command::new(bin())
        .args(["run"])
        .arg(target.join("duhem.yml"))
        .args(["--db"])
        .arg(&db)
        .args(["--reporter", "json"])
        .output()
        .expect("spawn run");
    assert!(
        run.status.success(),
        "run stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8(run.stdout).expect("utf8 stdout");
    let line = stdout.trim_end_matches('\n');
    let v: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
    assert_eq!(
        v["verdict"], "pass",
        "scaffolded VD should pass against example.com: {line}"
    );
}
