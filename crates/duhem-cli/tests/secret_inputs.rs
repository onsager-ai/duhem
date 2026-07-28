//! Black-box terminal-sink coverage for secret inputs (spec #346).
//!
//! The engine/evidence suites pin stored output. These tests exercise
//! the CLI presentations themselves so a future renderer cannot bypass
//! the registry while the database remains safe.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_duhem"))
}

fn fixture(
    tmp: &tempfile::TempDir,
    criterion_id: &str,
    description: &str,
    assertion: &str,
) -> PathBuf {
    let path = tmp.path().join("secret.yml");
    std::fs::write(
        &path,
        format!(
            r#"verification: secret terminal
inputs:
  api_token: {{ type: string, env: API_TOKEN, secret: true }}
criteria:
  - id: {criterion_id:?}
    description: {description:?}
    checks:
      - id: AC-1.1
        assertions: [{assertion:?}]
"#
        ),
    )
    .unwrap();
    path
}

#[test]
fn dry_run_masks_resolved_secret_input() {
    let tmp = tempfile::tempdir().unwrap();
    let secret = "0123456789abcdef0123456789abcdef";
    let path = fixture(&tmp, "AC-1", "terminal stays safe", "true");
    let output = Command::new(bin())
        .arg("run")
        .arg(path)
        .arg("--dry-run")
        .env("API_TOKEN", secret)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains(secret));
    assert!(stdout.contains("[redacted:api_token]"));
}

#[test]
fn live_progress_masks_secret_in_event_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let secret = "0123456789abcdef0123456789abcdef";
    let path = fixture(&tmp, secret, "terminal stays safe", "true");
    let output = Command::new(bin())
        .arg("run")
        .arg(path)
        .arg("--live")
        .arg("--db")
        .arg(tmp.path().join("duhem.db"))
        .env("API_TOKEN", secret)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains(secret));
    assert!(
        stderr.contains("[redacted:api_token]"),
        "masked live output was: {stderr:?}"
    );
}

#[test]
fn describe_and_validate_never_read_or_print_secret_env_values() {
    let tmp = tempfile::tempdir().unwrap();
    let secret = "0123456789abcdef0123456789abcdef";
    let path = fixture(&tmp, "AC-1", "validation does not resolve secrets", "true");
    for args in [
        vec!["describe".to_string(), "api/call".to_string()],
        vec!["validate".to_string(), path.display().to_string()],
    ] {
        let output = Command::new(bin())
            .args(args)
            .env("API_TOKEN", secret)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!String::from_utf8(output.stdout).unwrap().contains(secret));
        assert!(!String::from_utf8(output.stderr).unwrap().contains(secret));
    }
}
