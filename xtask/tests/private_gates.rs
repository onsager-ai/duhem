//! Exercise gate boundaries without mutating the test process's environment.
use std::process::Command;

fn xtask() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command.env("CARGO_MANIFEST_DIR", env!("CARGO_MANIFEST_DIR"));
    command
}

#[test]
fn gates_fail_explicitly_without_private_vocabulary() {
    for gate in ["source-scrub", "skill-scrub", "dx-drift", "tracker-scrub"] {
        let output = xtask()
            .arg(gate)
            .env_remove("DUHEM_PRIVATE_NAMES")
            .output()
            .unwrap();
        assert!(!output.status.success(), "{gate} silently passed");
        assert!(String::from_utf8_lossy(&output.stderr).contains("DUHEM_PRIVATE_NAMES"));
    }
}

#[test]
fn source_hit_reports_locations_without_secret_text() {
    // Fake names intentionally occur in tracked test fixtures and policy tests.
    let output = xtask()
        .arg("source-scrub")
        .env("DUHEM_PRIVATE_NAMES", "restricted-example")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("matching locations"));
    assert!(stdout.contains("xtask/"));
    assert!(!stdout.to_lowercase().contains("restricted-example"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("restricted-example"));
}

#[test]
fn consumer_pin_entry_point_accepts_a_pin_and_rejects_invalid_input() {
    let output = xtask()
        .args(["consumer-pins", "--pinned", "0.5.1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("::warning::consumer-pins"));
    for args in [
        vec!["consumer-pins"],
        vec!["consumer-pins", "--pinned", "latest"],
    ] {
        assert!(!xtask().args(args).output().unwrap().status.success());
    }
}
