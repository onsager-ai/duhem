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
fn semantic_reference_after_multi_step_flow_uses_authored_step_location() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("bad-after-flow.yml");
    std::fs::write(
        &leaf,
        r#"verification: bad reference after flow
flows:
  login:
    steps:
      - id: open
        uses: api/call
        with:
          url: /login
      - id: submit
        uses: api/call
        with:
          url: /session
criteria:
  - id: AC-1
    description: locate an error after expansion
    checks:
      - id: AC-1.1
        steps:
          - id: before
            uses: api/call
            with:
              url: /before
          - id: authenticate
            call: login
          - id: after
            uses: api/call
            with:
              url: $inputs.MISSING_AFTER
        assertions: ["true"]
"#,
    )
    .unwrap();

    let output = invoke("validate", &leaf);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.starts_with(&format!("{}:28:20: ", leaf.display())),
        "step after a multi-step flow should retain its authored mark: {message}"
    );
}

#[test]
fn semantic_reference_after_one_step_flow_keeps_exact_location() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("bad-after-one-step-flow.yml");
    std::fs::write(
        &leaf,
        r#"verification: bad reference after one-step flow
flows:
  prepare:
    steps:
      - id: prepare
        uses: api/call
        with:
          url: /prepare
criteria:
  - id: AC-1
    description: retain an existing exact location
    checks:
      - id: AC-1.1
        steps:
          - id: invoke
            call: prepare
          - id: fault
            uses: api/call
            with:
              url: $inputs.MISSING_ONE_STEP
        assertions: ["true"]
"#,
    )
    .unwrap();

    let output = invoke("validate", &leaf);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.starts_with(&format!("{}:20:20: ", leaf.display())),
        "one-step flow should preserve the already-working exact mark: {message}"
    );
}

#[test]
fn repeated_flow_errors_use_the_shared_definition_location() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("bad-inside-repeated-flow.yml");
    std::fs::write(
        &leaf,
        r#"verification: bad reference inside repeated flow
flows:
  inspect:
    steps:
      - id: lookup
        uses: api/call
        with:
          url: $pages.MISSING.target
criteria:
  - id: AC-1
    description: locate the shared authored expression
    checks:
      - id: AC-1.1
        steps:
          - id: first
            call: inspect
          - id: second
            call: inspect
        assertions: ["true"]
"#,
    )
    .unwrap();

    let output = invoke("validate", &leaf);
    assert!(!output.status.success());
    let message = stderr(&output);
    let diagnostics: Vec<_> = message.lines().collect();
    assert_eq!(
        diagnostics.len(),
        2,
        "each invocation should fail: {message}"
    );
    for diagnostic in diagnostics {
        assert!(
            diagnostic.starts_with(&format!("{}:8:16: ", leaf.display())),
            "each invocation should point to the shared flow definition: {diagnostic}"
        );
    }
}

#[test]
fn unprovable_aliased_step_location_falls_back_to_filename() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("bad-aliased-step.yml");
    std::fs::write(
        &leaf,
        r#"verification: bad aliased step
flows:
  templates:
    steps:
      - &shared_step
        id: shared
        uses: api/call
        with:
          url: $pages.MISSING.target
criteria:
  - id: AC-1
    description: do not invent an alias location
    checks:
      - id: AC-1.1
        steps:
          - *shared_step
        assertions: ["true"]
"#,
    )
    .unwrap();

    let output = invoke("validate", &leaf);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.starts_with(&format!("{}: [schema v", leaf.display())),
        "an alias without a provable scalar mark must use filename fallback: {message}"
    );
    assert!(
        !message.starts_with(&format!("{}:9:16: ", leaf.display())),
        "the anchor's location would be wrong for the aliased check step: {message}"
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
