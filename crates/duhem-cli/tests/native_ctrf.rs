//! CTRF reports produced by real shell commands use the common evidence path.
#![cfg(unix)]
use std::process::Command;

const FIXTURE: &str = include_str!("../../duhem-runtime/tests/fixtures/ctrf/results.json");

fn execute(report: &str) -> (serde_json::Value, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("input.json"), report).unwrap();
    std::fs::write(tmp.path().join("leaf.yml"), "verification: ctrf\ninputs:\n  token: {type: string, secret: true}\nnative:\n  - id: unit\n    layer: unit\n    command: [sh, -c, 'cat input.json > report.json']\n    report: {format: ctrf, path: report.json}\ncriteria:\n  - id: AC\n    description: Independent\n    checks:\n      - id: body\n        assertions: ['true']\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .args([
            "run",
            "--reporter",
            "json",
            "--no-live",
            "--inputs",
            "token=ctrf-secret-value",
            "--db",
        ])
        .arg(tmp.path().join("evidence.db"))
        .arg(tmp.path().join("leaf.yml"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let json = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (json, tmp)
}

#[tokio::test]
async fn counts_failures_and_masked_evidence() {
    use duhem_evidence::{SqliteStore, Store};
    let (json, tmp) = execute(FIXTURE);
    let native = &json["native"][0];
    assert_eq!(native["outcome"], "fail");
    assert_eq!(native["tests"], 5);
    assert_eq!(
        native["counts"],
        serde_json::json!({"passed": 1, "failed": 1, "errored": 1, "skipped": 2})
    );
    assert_eq!(native["failing_tests"].as_array().unwrap().len(), 2);
    assert_eq!(json["criteria"][0]["verdict"], "pass");
    assert!(!json.to_string().contains("ctrf-secret-value"));
    assert!(
        native["failing_tests"][0]["message"]
            .as_str()
            .unwrap()
            .contains("[redacted:token]")
    );
    let store = SqliteStore::open(tmp.path().join("evidence.db"))
        .await
        .unwrap();
    for field in ["report_blob_sha256", "results_blob_sha256"] {
        let blob = store
            .get_blob(native[field].as_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(blob).unwrap();
        assert!(!text.contains("ctrf-secret-value"));
        assert!(text.contains("[redacted:token]"));
        if field == "results_blob_sha256" {
            let tests: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(tests[0]["duration_ms"], 125);
            assert_eq!(tests[0]["file"], "tests/example.js");
            assert!(
                tests[1]["message"]
                    .as_str()
                    .unwrap()
                    .contains("at example.js:12")
            );
        }
    }
}

#[test]
fn malformed_or_future_report_is_inconclusive() {
    for report in [
        "{".to_string(),
        FIXTURE.replace("passed", "unknown"),
        FIXTURE.replace("0.2.0", "1.0.0"),
    ] {
        let (json, _tmp) = execute(&report);
        assert_eq!(
            json["native"][0]["outcome"],
            "inconclusive:missing_observation"
        );
        if report.contains("1.0.0") {
            assert!(
                json["native"][0]["detail"]
                    .as_str()
                    .unwrap()
                    .contains("newer than pinned major")
            );
        }
    }
}
