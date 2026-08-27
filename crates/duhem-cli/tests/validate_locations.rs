use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_duhem"))
}

fn invoke(command: &str, path: &Path) -> Output {
    Command::new(bin())
        .arg(command)
        .arg(path)
        .output()
        .expect("spawn duhem")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("utf-8 stderr")
}

#[test]
fn semantic_reference_error_points_at_exact_with_value() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("bad-semantic.yml");
    std::fs::write(
        &leaf,
        r#"verification: bad semantic
criteria:
  - id: AC-1
    description: undeclared input
    checks:
      - id: AC-1.1
        steps:
          - id: home
            uses: api/call
            with:
              url: $inputs.APP_BASE_URL
        assertions: ["true"]
"#,
    )
    .unwrap();

    let validate = invoke("validate", &leaf);
    assert!(!validate.status.success());
    let message = stderr(&validate);
    assert_eq!(
        message,
        format!(
            "{}:11:20: [schema v{}] validation error: criterion `AC-1` / check `AC-1.1`: step `home` with: `$inputs.APP_BASE_URL` references undeclared input `APP_BASE_URL`\n",
            leaf.display(),
            duhem_schema::SCHEMA_VERSION,
        )
    );

    // Validation happens before browser launch, so `run` exercises the same
    // real CLI path without requiring a browser installation.
    let run = invoke("run", &leaf);
    assert!(!run.status.success());
    assert_eq!(
        stderr(&run),
        message,
        "run and validate must emit the identical diagnostic prefix and prose"
    );
}

#[test]
fn semantic_reference_error_points_at_exact_assertion_string() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("bad-assertion.yml");
    std::fs::write(
        &leaf,
        r#"verification: bad assertion
criteria:
  - id: AC-1
    description: undeclared input
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.MISSING == "expected"
"#,
    )
    .unwrap();

    let output = invoke("validate", &leaf);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.starts_with(&format!("{}:8:13: ", leaf.display())),
        "assertion scalar should carry its exact mark: {message}"
    );
    assert!(
        message.contains("criterion `AC-1` / check `AC-1.1`: assertion"),
        "logical prose must survive beside the location: {message}"
    );
}

#[test]
fn unlocated_semantic_error_falls_back_to_filename() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("empty.yml");
    std::fs::write(&leaf, "verification: empty\ncriteria: []\n").unwrap();

    let output = invoke("validate", &leaf);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.starts_with(&format!("{}: [schema v", leaf.display())),
        "fallback should retain the filename: {message}"
    );
    assert!(
        !message.contains(":0:0:"),
        "zero span is fabricated: {message}"
    );
}

#[test]
fn yaml_parse_error_keeps_existing_location_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("bad-parse.yml");
    std::fs::write(
        &leaf,
        "verification: bad parse\ncriteria:\n  - id: AC-1\n    :: nope\n",
    )
    .unwrap();

    let output = invoke("validate", &leaf);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.starts_with(&format!(
            "{}:4:5: [schema v{}] YAML parse error:",
            leaf.display(),
            duhem_schema::SCHEMA_VERSION,
        )),
        "parse diagnostic changed: {message}"
    );
}

#[test]
fn unknown_action_is_rejected_at_uses_location_before_run() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("unknown-action.yml");
    std::fs::write(
        &leaf,
        r#"verification: unknown action
criteria:
  - id: AC-1
    description: typo
    checks:
      - id: AC-1.1
        steps:
          - id: wait_two_seconds
            uses: ui/wait-nonexistent
"#,
    )
    .unwrap();

    for command in ["validate", "run"] {
        let output = invoke(command, &leaf);
        assert!(!output.status.success());
        let message = stderr(&output);
        assert!(
            message.starts_with(&format!("{}:9:19: ", leaf.display())),
            "{message}"
        );
        assert!(
            message.contains("unknown action `ui/wait-nonexistent`"),
            "{message}"
        );
        assert!(message.contains("see `duhem actions`"), "{message}");
    }
}
