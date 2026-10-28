//! Executable examples for required and diagnostic fixture cleanup (#567).

use std::path::Path;
use std::process::Command;

#[test]
fn fixture_cleanup_policy_controls_cli_gate() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cleanup");
    for (file, passes) in [("required.yml", false), ("diagnostic.yml", true)] {
        let tmp = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
            .arg("run")
            .arg(fixtures.join(file))
            .arg("--db")
            .arg(tmp.path().join("duhem.db"))
            .arg("--no-live")
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            passes,
            "{file}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}
