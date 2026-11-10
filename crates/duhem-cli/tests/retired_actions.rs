use std::process::Command;

#[test]
fn assert_state_is_rejected_while_polling_and_selection_remain_catalogued() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("retired.yml");
    std::fs::write(&path, "verification: retired\ncriteria:\n  - id: AC\n    description: reject retired action\n    checks:\n      - id: C\n        steps: [{uses: ui/assert-state, with: {state: loaded}}]\n        assertions: ['true']\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .arg("validate")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success(), "retired action was accepted");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("ui/assert-state") && error.contains("unknown"),
        "{error}"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .arg("actions")
        .output()
        .unwrap();
    assert!(output.status.success());
    let catalog = String::from_utf8(output.stdout).unwrap();
    assert!(!catalog.contains("ui/assert-state"), "{catalog}");
    for action in ["ui/assert-element", "api/poll", "db/observe", "ui/select"] {
        assert!(catalog.contains(action), "missing {action}: {catalog}");
    }
}
