use duhem_evidence::{EvidenceWriter, SqliteStore};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use std::{path::Path, process::Command};

const LEAF: &str = "verification: same title\ncriteria:\n  - id: AC\n    description: pass\n    checks:\n      - id: C\n        assertions: ['true']\n";

fn coverage(path: &Path, db: &Path, json: bool) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_duhem"));
    cmd.arg("coverage").arg(path).arg("--db").arg(db);
    if json {
        cmd.args(["--format", "json"]);
    }
    cmd.output().unwrap()
}

#[test]
fn directory_manifest_and_leaf_reports_use_source_identity_and_only_the_supplied_store() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path();
    std::fs::create_dir(path.join("suite")).unwrap();
    let seen = path.join("suite/seen.yml");
    let unseen = path.join("suite/unseen.yml");
    std::fs::write(&seen, LEAF).unwrap();
    std::fs::write(&unseen, LEAF).unwrap();
    let manifest = path.join("suite/duhem.yml");
    std::fs::write(
        &manifest,
        "manifest_version: 1\nverifications:\n  - path: seen.yml\n  - path: unseen.yml\n",
    )
    .unwrap();
    let db = path.join("local.db");
    let other = path.join("other.db");
    for (leaf, store) in [(&seen, &db), (&unseen, &other)] {
        let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
            .arg("run")
            .arg(leaf)
            .arg("--db")
            .arg(store)
            .args(["--reporter", "quiet", "--no-live"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for target in [path.to_path_buf(), manifest.clone()] {
        let output = coverage(&target, &db, true);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["definitions"].as_array().unwrap().len(), 2);
        let definitions = report["definitions"].as_array().unwrap();
        assert_eq!(definitions[0]["status"], "recorded_run");
        assert!(definitions[0]["last_recorded_run"]["started_at"].is_string());
        assert_eq!(definitions[0]["last_recorded_run"]["verdict"], "pass");
        assert_eq!(definitions[1]["status"], "no_recorded_run");
        assert!(definitions[1]["last_recorded_run"].is_null());
        assert!(
            report["scope"]
                .as_str()
                .unwrap()
                .contains("Only runs recorded in this supplied store")
        );
    }
    let output = coverage(&unseen, &db, false);
    assert!(output.status.success(), "absence is not a failing gate");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("1 of 1 definitions have no recorded run"),
        "{text}"
    );
    assert!(text.contains("not proof that an invocation never happened"));
    assert!(text.contains(&db.display().to_string()));
    let missing = path.join("missing.db");
    assert_eq!(coverage(&seen, &missing, false).status.code(), Some(2));
    assert!(!missing.exists(), "reporting must not create a store");
    let empty = path.join("empty");
    std::fs::create_dir(&empty).unwrap();
    assert_eq!(coverage(&empty, &db, false).status.code(), Some(2));
}

#[tokio::test]
async fn relative_records_are_not_guessed_and_unfinished_runs_still_count_as_recorded() {
    let temp = tempfile::tempdir().unwrap();
    let leaf = temp.path().join("duhem.yml");
    std::fs::write(&leaf, LEAF).unwrap();
    let db = temp.path().join("runs.db");
    let store = Arc::new(SqliteStore::open(&db).await.unwrap());
    let writer = EvidenceWriter::begin(store.clone(), "relative", "duhem.yml", BTreeMap::new())
        .await
        .unwrap();
    drop(writer);
    let first: Value = serde_json::from_slice(&coverage(&leaf, &db, true).stdout).unwrap();
    assert_eq!(first["definitions"][0]["status"], "no_recorded_run");
    let writer = EvidenceWriter::begin(
        store.clone(),
        "absolute",
        leaf.to_str().unwrap(),
        BTreeMap::new(),
    )
    .await
    .unwrap();
    drop(writer);
    let second: Value = serde_json::from_slice(&coverage(&leaf, &db, true).stdout).unwrap();
    assert_eq!(second["definitions"][0]["status"], "recorded_run");
    assert_eq!(
        second["definitions"][0]["last_recorded_run"]["run_id"],
        "absolute"
    );
    assert!(second["definitions"][0]["last_recorded_run"]["verdict"].is_null());
}
