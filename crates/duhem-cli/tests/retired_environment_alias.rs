use std::process::Command;

#[test]
fn both_commands_reject_the_retired_alias_before_loading_the_definition() {
    for command in ["run", "resolve"] {
        let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
            .args([command, "not-a-file.yml", "--environment", "prod"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("unexpected argument '--environment'"),
            "{error}"
        );
        let help = Command::new(env!("CARGO_BIN_EXE_duhem"))
            .args([command, "--help"])
            .output()
            .unwrap();
        let help = String::from_utf8(help.stdout).unwrap();
        assert!(help.contains("--profile"));
        assert!(!help.contains("--environment"));
    }
}
