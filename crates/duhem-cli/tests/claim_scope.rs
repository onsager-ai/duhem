use std::path::Path;
use std::process::Command;

use serde_json::Value;

const LEAF: &str = "verification: claim scope\ncriteria:\n  - id: AC\n    description: two independent claims\n    checks:\n      - id: first\n        assertions: ['true']\n      - id: second\n        assertions: ['true']\n";

fn run(path: &Path, db: &Path, filters: &[&str], reporter: &str) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_duhem"));
    command
        .args(["run", "--no-live", "--reporter", reporter, "--db"])
        .arg(db)
        .arg(path);
    for filter in filters {
        command.args(["--filter", filter]);
    }
    command.output().unwrap()
}

fn json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
    })
}

#[tokio::test]
async fn filtered_pass_and_complete_pass_have_distinct_recorded_claim_sets() {
    use duhem_evidence::{SqliteStore, Store};
    let temp = tempfile::tempdir().unwrap();
    let leaf = temp.path().join("case.yml");
    std::fs::write(&leaf, LEAF).unwrap();
    for (filters, selected) in [(vec![], 2), (vec!["AC::first"], 1), (vec!["missing"], 0)] {
        let db = temp.path().join(format!("{selected}.db"));
        let output = run(&leaf, &db, &filters, "json");
        let report = json(&output);
        let scope = &report["claim_scope"];
        assert_eq!(scope["checks"]["authored"], 2);
        assert_eq!(scope["checks"]["selected"], selected);
        assert_eq!(scope["checks"]["executed"], selected);
        assert_eq!(scope["criteria"]["authored"], 1);
        assert_eq!(scope["criteria"]["executed"], u32::from(selected > 0));
        assert_eq!(output.status.success(), selected > 0, "{report}");
        if filters.is_empty() {
            assert!(scope.get("filters").is_none());
            assert!(scope.get("excluded").is_none());
        } else {
            assert_eq!(scope["filters"], serde_json::json!(filters));
            assert_eq!(
                scope["excluded"].as_array().unwrap().len(),
                2 - selected as usize
            );
        }
        let store = SqliteStore::open(&db).await.unwrap();
        let events = store
            .run_events(report["run_id"].as_str().unwrap())
            .await
            .unwrap();
        let recorded = duhem_evidence::claim_scope::claim_scope(&events).unwrap();
        assert_eq!(serde_json::to_value(recorded).unwrap(), *scope);
        let trace = duhem_evidence::Trace::from_events(events).unwrap();
        let replayed = duhem_evidence::replay(&trace).unwrap();
        assert_eq!(
            replayed.run.state.to_string(),
            report["verdict"].as_str().unwrap()
        );
    }
    let output = run(
        &leaf,
        &temp.path().join("text.db"),
        &["AC::first"],
        "default",
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("1 of 2 checks recorded"), "{text}");
    assert!(
        text.contains("excluded AC::second (check_filter)"),
        "{text}"
    );
}

#[test]
fn manifest_directory_and_verification_filters_retain_the_authored_leaf_population() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("selected")).unwrap();
    std::fs::write(temp.path().join("selected/alpha.yml"), LEAF).unwrap();
    std::fs::write(temp.path().join("beta.yml"), LEAF).unwrap();
    let manifest = temp.path().join("duhem.yml");
    std::fs::write(
        &manifest,
        "manifest_version: 1\nverifications:\n  - path: selected/alpha.yml\n  - path: beta.yml\n",
    )
    .unwrap();
    for (target, filters, count, reason) in [
        (manifest.clone(), vec![], 2, None),
        (
            temp.path().join("selected"),
            vec![],
            1,
            Some("directory_selection"),
        ),
        (
            manifest.clone(),
            vec!["alpha::AC::first"],
            1,
            Some("verification_filter"),
        ),
        (
            manifest.clone(),
            vec!["absent::AC::first"],
            0,
            Some("verification_filter"),
        ),
    ] {
        let output = run(
            &target,
            &temp
                .path()
                .join(format!("{count}-{}.db", reason.unwrap_or("complete"))),
            &filters,
            "json",
        );
        let report = json(&output);
        assert_eq!(report["selection"]["authored"], 2);
        assert_eq!(report["selection"]["selected"], count);
        assert_eq!(report["runs"].as_array().unwrap().len(), count as usize);
        if let Some(reason) = reason {
            assert_eq!(
                report["selection"]["excluded"].as_array().unwrap().len(),
                2 - count as usize
            );
            assert_eq!(report["selection"]["excluded"][0]["reason"], reason);
        }
        for leaf in report["runs"].as_array().unwrap() {
            assert_eq!(leaf["claim_scope"]["manifest"], report["selection"]);
        }
        assert_eq!(output.status.success(), count > 0, "{report}");
    }
}
