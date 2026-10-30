use std::process::Command;

#[test]
fn retired_actions_are_rejected_by_catalog_validation_and_wait_remains_usable() {
    let temp = tempfile::tempdir().unwrap();
    for action in ["ui/select", "ui/assert-state", "api/poll", "db/observe"] {
        let path = temp.path().join("case.yml");
        std::fs::write(&path, format!("verification: retired\ncriteria:\n  - id: AC\n    description: reject\n    checks:\n      - id: C\n        steps: [{{uses: {action}}}]\n        assertions: ['true']\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
            .arg("validate")
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "retired action {action} was accepted"
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains(action) && error.contains("unknown"),
            "{error}"
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .arg("actions")
        .output()
        .unwrap();
    let catalog = String::from_utf8(output.stdout).unwrap();
    for action in ["ui/select", "ui/assert-state", "api/poll", "db/observe"] {
        assert!(!catalog.contains(action), "{catalog}");
    }
    assert!(catalog.contains("ui/wait"));
    let path = temp.path().join("wait.yml");
    std::fs::write(&path, "verification: retained wait\ncriteria:\n  - id: AC\n    description: wait is available\n    checks:\n      - id: C\n        steps: [{uses: ui/wait, with: {duration: 1ms}}]\n        assertions: ['true']\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .arg("run")
        .arg(&path)
        .arg("--db")
        .arg(temp.path().join("runs.db"))
        .args(["--no-live", "--reporter", "quiet"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
