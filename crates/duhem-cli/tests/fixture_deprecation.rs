//! `fixtures:` is deprecated in favour of flow `cleanup:` (#618): `duhem
//! validate` and `duhem run` warn once per declared fixture, a flow-only
//! definition warns nothing, and the deprecated form keeps working.

use std::path::Path;
use std::process::{Command, Output};

const FIXTURE_VD: &str = "verification: deprecated fixture
fixtures:
  thing:
    up:
      - id: make
        uses: cli/invoke
        with: { command: [echo, made] }
    down:
      - uses: cli/invoke
        with: { command: [echo, $fixture.thing.make.outputs.stdout] }
criteria:
  - id: AC-1
    description: a fixture is consumed
    checks:
      - id: AC-1.1
        needs: [thing]
        steps: [{ uses: cli/invoke, with: { command: [echo, ok] } }]
        assertions: [\"true\"]
";

const FLOW_VD: &str = "verification: flow cleanup
flows:
  thing:
    steps:
      - id: make
        uses: cli/invoke
        with: { command: [echo, made] }
    cleanup:
      - uses: cli/invoke
        with: { command: [echo, $steps.make.outputs.stdout] }
criteria:
  - id: AC-1
    description: a flow with cleanup is called
    checks:
      - id: AC-1.1
        setup: [{ call: thing }]
        steps: [{ uses: cli/invoke, with: { command: [echo, ok] } }]
        assertions: [\"true\"]
";

fn duhem(args: &[&str], leaf: &Path, db: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_duhem"));
    command.args(args).arg(leaf);
    if let Some(db) = db {
        command.args(["--no-live", "--db"]).arg(db);
    }
    command.output().unwrap()
}

fn leaf(yaml: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("leaf.yml");
    std::fs::write(&path, yaml).unwrap();
    (tmp, path)
}

fn assert_deprecation_warning(stderr: &str) {
    let line = stderr
        .lines()
        .find(|line| line.contains("`fixtures.thing` is deprecated and will be removed in v0.9.0"))
        .unwrap_or_else(|| panic!("no deprecation warning: {stderr}"));
    assert!(line.starts_with("warning: "), "{line}");
    assert!(
        line.contains("`needs: [thing]` with `- call: thing` at the end of the check's `setup:`"),
        "{line}"
    );
    // A source location, as `line:column:`, right before the warning text.
    let prefix = &line[..line.find("`fixtures.thing`").unwrap()];
    let location = prefix
        .trim_end()
        .trim_end_matches(':')
        .rsplit(' ')
        .next()
        .unwrap();
    let (line_no, column) = location
        .split_once(':')
        .unwrap_or_else(|| panic!("no location in `{line}`"));
    assert!(
        [line_no, column]
            .iter()
            .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())),
        "{line}"
    );
    assert_eq!(
        stderr.matches("is deprecated and will be removed").count(),
        1,
        "{stderr}"
    );
}

#[test]
fn validate_accepts_a_fixture_and_warns_with_a_location_and_the_rewrite() {
    let (_tmp, path) = leaf(FIXTURE_VD);
    let output = duhem(&["validate"], &path, None);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert_deprecation_warning(&stderr);
}

#[test]
fn validate_of_a_flow_only_definition_warns_nothing() {
    let (_tmp, path) = leaf(FLOW_VD);
    let output = duhem(&["validate"], &path, None);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("deprecated"), "{stderr}");
}

#[test]
fn run_warns_for_a_fixture_and_still_passes() {
    let (tmp, path) = leaf(FIXTURE_VD);
    let output = duhem(&["run"], &path, Some(&tmp.path().join("duhem.db")));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert_deprecation_warning(&stderr);
}

#[test]
fn run_of_a_flow_only_definition_warns_nothing() {
    let (tmp, path) = leaf(FLOW_VD);
    let output = duhem(&["run"], &path, Some(&tmp.path().join("duhem.db")));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("deprecated"), "{stderr}");
}
