//! Removed authoring forms fail with a located migration error (#618).
use std::process::Command;

#[test]
fn validate_rejects_every_removed_surface_with_the_rewrite_and_location() {
    for (yaml, line, column) in [
        ("verification: x\nfixtures: {}\ncriteria: []\n".to_string(), 2, 1),
        ("verification: x\ncriteria:\n  - id: AC\n    description: x\n    checks:\n      - id: c\n        needs: [res]\n        assertions: [\"true\"]\n".to_string(), 7, 9),
        ("verification: x\nsetup:\n  - uses: cli/invoke\n    needs: [res]\ncriteria: []\n".to_string(), 4, 5),
        ("verification: x\nsetup:\n  - uses: cli/invoke\n    with:\n      command: $fixture.res.make.outputs.stdout\ncriteria: []\n".to_string(), 5, 16),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("leaf.yml");
        std::fs::write(&path, yaml).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
            .arg("validate").arg(&path).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{stderr}");
        assert!(stderr.contains(&format!("{}:{line}:{column}:", path.display())), "{stderr}");
        for text in [
            "removed in v0.9.0",
            "`flows.<name>` with `steps:` (was `up:`), `cleanup:` (was `down:`)",
            "same `cleanup_policy:`",
            "`$fixture.<name>.<id>.outputs.*` with `$steps.<id>.outputs.*`",
            "`needs: [<name>]` with `- call: <name>` at the end of the check's `setup:`",
        ] { assert!(stderr.contains(text), "{stderr}"); }
    }
}

#[test]
fn flow_only_definition_validates_and_runs_without_deprecation() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("leaf.yml");
    std::fs::write(&path, "verification: flow cleanup\nflows:\n  res:\n    steps: [{uses: cli/invoke, with: {command: [echo, made]}}]\n    cleanup: [{uses: cli/invoke, with: {command: [echo, done]}}]\ncriteria:\n  - id: AC\n    description: Flow cleanup\n    checks:\n      - id: c\n        setup: [{call: res}]\n        assertions: [\"true\"]\n").unwrap();
    for command in ["validate", "run"] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_duhem"));
        cmd.arg(command).arg(&path);
        if command == "run" {
            cmd.arg("--no-live")
                .arg("--db")
                .arg(tmp.path().join("run.db"));
        }
        let output = cmd.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(!stderr.contains("deprecated"), "{stderr}");
    }
}
