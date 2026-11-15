//! Real subprocesses prove native results gate runs without satisfying criteria.
#![cfg(unix)]
use std::process::Command;

const BODY: &str = "criteria:\n  - id: AC\n    description: Independent body\n    checks:\n      - id: body\n        assertions: ['true']\n";

fn execute(native: &str, manifest: bool) -> (bool, serde_json::Value, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("leaf.yml");
    std::fs::write(
        &leaf,
        format!(
            "verification: native-example\n{}{BODY}",
            if manifest { "" } else { native }
        ),
    )
    .unwrap();
    let path = if manifest {
        let root = tmp.path().join("duhem.yml");
        std::fs::write(
            &root,
            format!("manifest_version: 1\n{native}verifications: [{{path: leaf.yml}}]\n"),
        )
        .unwrap();
        root
    } else {
        leaf
    };
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .args(["run", "--reporter", "json", "--no-live", "--db"])
        .arg(tmp.path().join("evidence.db"))
        .arg(path)
        .output()
        .unwrap();
    let json = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.success(), json, tmp)
}

#[test]
fn exit_codes_output_matching_timeout_and_spawn_failure() {
    for (fields, outcome, success) in [
        ("command: [sh, -c, 'exit 0']", "pass", true),
        ("command: [sh, -c, 'exit 9']", "fail", false),
        (
            "command: [sh, -c, 'exit 9'], expect: {exit_code: 9}",
            "pass",
            true,
        ),
        (
            "command: [sh, -c, 'echo warning >&2'], expect: {stderr: {not_matches: warning}}",
            "fail",
            false,
        ),
        (
            "command: [sh, -c, 'echo hello'], expect: {stdout: {matches: '^hello'}}",
            "pass",
            true,
        ),
        (
            "command: [sh, -c, 'sleep 1'], timeout: 10ms",
            "inconclusive:timeout",
            false,
        ),
        (
            "command: [duhem-native-nonexistent-command]",
            "inconclusive:environment_error",
            false,
        ),
    ] {
        let (passed, json, _tmp) = execute(
            &format!("native: [{{id: tool, layer: static, {fields}}}]\n"),
            false,
        );
        assert_eq!(passed, success, "{json}");
        assert_eq!(json["native"][0]["outcome"], outcome);
        assert_eq!(json["native"][0]["tests"], 1);
        assert_eq!(json["criteria"][0]["verdict"], "pass");
        assert!(json["native"][0]["duration_ms"].is_number());
        if !success {
            assert_eq!(json["body_verdict"], "pass");
        }
    }
}

#[tokio::test]
async fn required_and_diagnostic_runs_gate_leaf_and_manifest() {
    use duhem_evidence::{SqliteStore, Store, Trace, replay};
    for manifest in [false, true] {
        for required in [false, true] {
            let (success, json, tmp) = execute(
                &format!(
                    "native: [{{id: lint, command: [sh, -c, 'exit 3'], required: {required}}}]\n"
                ),
                manifest,
            );
            assert_eq!(success, !required, "{json}");
            assert_eq!(json["verdict"], if required { "fail" } else { "pass" });
            assert_eq!(json["native"][0]["outcome"], "fail");
            let body = if manifest { &json["runs"][0] } else { &json };
            assert_eq!(body["criteria"][0]["verdict"], "pass");
            let store = SqliteStore::open(tmp.path().join("evidence.db"))
                .await
                .unwrap();
            let trace = Trace::from_store(&store, body["run_id"].as_str().unwrap())
                .await
                .unwrap();
            let replayed = replay(&trace).unwrap();
            assert_eq!(
                serde_json::to_value(replayed.run.state).unwrap(),
                body["verdict"]
            );
            let events = store
                .run_events(json["run_id"].as_str().unwrap())
                .await
                .unwrap();
            assert!(events.iter().any(|event| matches!(&event.payload, duhem_evidence::EventPayload::NativeRunFinished { result } if result.id == "lint")));
            assert_eq!(
                json["layers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| row["tests"].as_u64().unwrap())
                    .sum::<u64>(),
                2
            );
        }
    }
}

#[tokio::test]
async fn secret_output_is_masked_in_summary_and_blob() {
    use duhem_evidence::{SqliteStore, Store};
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("leaf.yml");
    std::fs::write(&leaf, format!("verification: secrets\ninputs:\n  token: {{type: string, secret: true}}\nnative:\n  - id: secret-output\n    command: [sh, -c, 'echo \"$TOKEN\"; exit 1']\n    env: {{TOKEN: native-secret-value}}\n{BODY}")).unwrap();
    let db = tmp.path().join("evidence.db");
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .args([
            "run",
            "--reporter",
            "json",
            "--no-live",
            "--inputs",
            "token=native-secret-value",
            "--db",
        ])
        .arg(&db)
        .arg(&leaf)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("native-secret-value"), "{text}");
    assert!(text.contains("[redacted:token]"), "{text}");
    let summary: serde_json::Value = serde_json::from_str(&text).unwrap();
    let store = SqliteStore::open(db).await.unwrap();
    let blob = store
        .get_blob(summary["native"][0]["stdout_blob_sha256"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(!String::from_utf8_lossy(&blob).contains("native-secret-value"));
}

#[test]
fn native_commands_execute_in_order_and_resolve_cwd() {
    let (success, json, tmp) = execute(
        "native:\n  - id: write\n    command: [sh, -c, 'printf first > marker']\n  - id: read\n    cwd: .\n    command: [sh, -c, 'cat marker']\n    expect: {stdout: {matches: '^first$'}}\n",
        true,
    );
    assert!(success, "{json}");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("marker")).unwrap(),
        "first"
    );
}
