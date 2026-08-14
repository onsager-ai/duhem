//! Manifest ordering regression for leaf `teardown:` (spec #409).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_duhem"))
}

fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, source).unwrap();
}

fn script(root: &Path, name: &str, marker: &Path) {
    let path = root.join(name);
    write(
        root,
        name,
        &format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    );
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn aborting_leaf_tears_down_before_suite_and_stops_remaining_leaves() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf1 = tmp.path().join("leaf-1-ran");
    let leaf2_setup = tmp.path().join("leaf-2-setup-ran");
    let leaf2_teardown = tmp.path().join("leaf-2-teardown-ran");
    let leaf3 = tmp.path().join("leaf-3-ran");
    let suite_up = tmp.path().join("suite-up-ran");
    let suite_down = tmp.path().join("suite-down-ran");
    let order = tmp.path().join("cleanup-order");
    script(tmp.path(), "suite-up.sh", &suite_up);
    let suite_down_script = tmp.path().join("suite-down.sh");
    write(
        tmp.path(),
        "suite-down.sh",
        &format!(
            "#!/bin/sh\ntouch '{}'\nprintf '%s\\n' suite-down >> '{}'\n",
            suite_down.display(),
            order.display()
        ),
    );
    std::fs::set_permissions(suite_down_script, std::fs::Permissions::from_mode(0o755)).unwrap();

    write(
        tmp.path(),
        "duhem.yml",
        r#"
manifest_version: 1
provision:
  up: ./suite-up.sh
  down: ./suite-down.sh
verifications:
  - path: leaves/one.yml
  - path: leaves/two.yml
  - path: leaves/three.yml
"#,
    );
    write(
        tmp.path(),
        "leaves/one.yml",
        &format!(
            r#"
verification: leaf one
criteria:
  - id: AC-1
    description: first leaf runs
    checks:
      - id: AC-1.1
        steps:
          - id: mark
            uses: cli/invoke
            with: {{ command: [sh, -c, "touch '{}'"] }}
        assertions: ["true"]
"#,
            leaf1.display()
        ),
    );
    write(
        tmp.path(),
        "leaves/two.yml",
        &format!(
            r#"
verification: leaf two
setup:
  - uses: cli/invoke
    with: {{ command: [sh, -c, "touch '{}'"] }}
teardown:
  - uses: cli/invoke
    with:
      command: [sh, -c, "touch '{}'; printf '%s\\n' leaf-2-teardown >> '{}'"]
criteria:
  - id: AC-1
    description: an engine error aborts this leaf
    checks:
      - id: AC-1.1
        steps:
          - id: missing
            uses: cli/invoke
            if: failure
            with: {{ command: [sh, -c, "true"] }}
          - id: abort
            uses: cli/invoke
            with: {{ command: [sh, -c, $steps.missing.outputs.stdout] }}
        assertions: ["true"]
"#,
            leaf2_setup.display(),
            leaf2_teardown.display(),
            order.display()
        ),
    );
    write(
        tmp.path(),
        "leaves/three.yml",
        &format!(
            r#"
verification: leaf three
criteria:
  - id: AC-1
    description: third leaf must not run
    checks:
      - id: AC-1.1
        steps:
          - uses: cli/invoke
            with: {{ command: [sh, -c, "touch '{}'"] }}
        assertions: ["true"]
"#,
            leaf3.display()
        ),
    );

    let output = Command::new(bin())
        .arg("run")
        .arg(tmp.path())
        .arg("--db")
        .arg(tmp.path().join("duhem.db"))
        .arg("--no-live")
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "leaf 2 must preserve its engine error"
    );
    assert!(leaf1.exists(), "leaf 1 ran before the abort");
    assert!(leaf2_setup.exists(), "leaf 2 setup dispatched");
    assert!(leaf2_teardown.exists(), "leaf 2 teardown drained first");
    assert!(
        suite_down.exists(),
        "shared suite teardown ran after leaf cleanup"
    );
    assert_eq!(
        std::fs::read_to_string(&order).unwrap(),
        "leaf-2-teardown\nsuite-down\n",
        "leaf teardown must finish before the shared stack comes down"
    );
    assert!(
        !leaf3.exists(),
        "the engine error still aborts remaining leaves"
    );
}

#[test]
fn teardown_failure_does_not_abort_remaining_manifest_leaves() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf2 = tmp.path().join("leaf-2-ran");
    write(
        tmp.path(),
        "duhem.yml",
        r#"
manifest_version: 1
verifications:
  - path: leaves/one.yml
  - path: leaves/two.yml
"#,
    );
    write(
        tmp.path(),
        "leaves/one.yml",
        r#"
verification: cleanup failure
setup:
  - uses: cli/invoke
    with: { command: [sh, -c, "true"] }
teardown:
  - id: broken-cleanup
    uses: synthetic/missing-action
criteria:
  - id: AC-1
    description: the verified behavior still passes
    checks:
      - id: AC-1.1
        steps:
          - id: check
            uses: cli/invoke
            with: { command: [sh, -c, "true"] }
        assertions: [$steps.check.outputs.exit_code == 0]
"#,
    );
    write(
        tmp.path(),
        "leaves/two.yml",
        &format!(
            r#"
verification: later leaf
criteria:
  - id: AC-1
    description: later leaves still run
    checks:
      - id: AC-1.1
        steps:
          - id: mark
            uses: cli/invoke
            with: {{ command: [sh, -c, "touch '{}'"] }}
        assertions: [$steps.mark.outputs.exit_code == 0]
"#,
            leaf2.display()
        ),
    );

    let output = Command::new(bin())
        .arg("run")
        .arg(tmp.path())
        .arg("--db")
        .arg(tmp.path().join("duhem.db"))
        .arg("--no-live")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cleanup evidence must not fail the manifest: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(leaf2.exists(), "the later leaf must still run");
}
