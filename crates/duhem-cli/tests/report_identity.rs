use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn run(path: &Path, database: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .args(["run", "--reporter", "json", "--no-live", "--db"])
        .arg(database)
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn each_manifest_result_identifies_its_own_leaf_independently_of_position_and_title() {
    let temp = tempfile::tempdir().unwrap();
    let names = ["alpha.yml", "beta.yml", "gamma.yml"];
    for name in names {
        std::fs::write(temp.path().join(name), "verification: same title\ncriteria:\n  - id: AC\n    description: pass\n    checks:\n      - id: C\n        assertions: ['true']\n").unwrap();
    }
    let manifest = temp.path().join("duhem.yml");
    std::fs::write(&manifest, "manifest_version: 1\nverifications:\n  - path: gamma.yml\n  - path: alpha.yml\n  - path: beta.yml\n").unwrap();
    let report = run(&manifest, &temp.path().join("set.db"));
    let identities: BTreeSet<_> = report["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["verification"].as_str().unwrap().to_string())
        .collect();
    let expected: BTreeSet<_> = names
        .iter()
        .map(|name| temp.path().join(name).display().to_string())
        .collect();
    assert_eq!(identities, expected);
    let run_ids: BTreeSet<_> = report["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["run_id"].as_str().unwrap())
        .collect();
    assert_eq!(run_ids.len(), 3);
    let single = run(
        &temp.path().join("beta.yml"),
        &temp.path().join("single.db"),
    );
    assert_eq!(
        single["verification"],
        temp.path().join("beta.yml").display().to_string()
    );
}
