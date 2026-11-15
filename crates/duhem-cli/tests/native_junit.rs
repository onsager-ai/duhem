//! Real shell commands write committed reports into the native cwd.
#![cfg(unix)]
use std::process::Command;

const BODY: &str = "criteria:\n  - id: AC\n    description: Independent\n    checks:\n      - id: body\n        assertions: ['true']\n";
const PASS: &str = include_str!("../../duhem-runtime/tests/fixtures/junit/pass.xml");
const MALFORMED: &str = include_str!("../../duhem-runtime/tests/fixtures/junit/malformed.xml");
const FAIL: &str = include_str!("../../duhem-runtime/tests/fixtures/junit/pytest.xml");

fn execute(
    xml: Option<&str>,
    exit: i32,
    extra: &str,
    secret: bool,
) -> (serde_json::Value, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("work")).unwrap();
    if let Some(xml) = xml {
        std::fs::write(tmp.path().join("work/input.xml"), xml).unwrap();
    }
    let command = if extra.contains("timeout:") {
        "sleep 1; cat input.xml > report.xml".to_string()
    } else if xml.is_some() {
        format!("cat input.xml > report.xml; exit {exit}")
    } else {
        format!("exit {exit}")
    };
    let inputs = if secret {
        "inputs:\n  token: {type: string, secret: true}\n"
    } else {
        ""
    };
    let leaf = tmp.path().join("leaf.yml");
    std::fs::write(&leaf, format!("verification: junit\n{inputs}native:\n  - id: unit\n    layer: unit\n    cwd: work\n    command: [sh, -c, '{command}']\n    report: {{format: junit, path: report.xml}}\n{extra}{BODY}")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_duhem"));
    command
        .args(["run", "--reporter", "json", "--no-live", "--db"])
        .arg(tmp.path().join("evidence.db"));
    if secret {
        command.args(["--inputs", "token=junit-secret-value"]);
    }
    let output = command.arg(&leaf).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(output.status.success(), json["verdict"] == "pass", "{json}");
    (json, tmp)
}

#[test]
fn junit_outcomes_and_counts() {
    for (xml, exit, extra, outcome) in [
        (Some(PASS), 0, "", "pass"),
        (Some(FAIL), 0, "", "fail"),
        (Some(PASS), 9, "", "fail"),
        (Some(PASS), 9, "    expect: {exit_code: 9}\n", "pass"),
        (
            Some(PASS),
            0,
            "    expect: {stdout: {matches: missing}}\n",
            "fail",
        ),
        (Some(MALFORMED), 0, "", "inconclusive:missing_observation"),
        (Some(MALFORMED), 1, "", "inconclusive:missing_observation"),
        (None, 0, "", "inconclusive:missing_observation"),
        (None, 1, "", "inconclusive:missing_observation"),
        (Some(PASS), 0, "    timeout: 10ms\n", "inconclusive:timeout"),
    ] {
        let (json, _tmp) = execute(xml, exit, extra, false);
        let native = &json["native"][0];
        assert_eq!(native["outcome"], outcome, "{json}");
        if xml.is_none() && exit != 0 {
            // A missing report keeps the exit-code evidence (e.g. a build failure).
            let detail = native["detail"].as_str().unwrap();
            assert!(detail.contains("cannot read JUnit report"), "{detail}");
            assert!(detail.contains("expected exit code 0"), "{detail}");
        }
        assert_eq!(json["criteria"][0]["verdict"], "pass");
        assert_eq!(native["report_path"], "report.xml");
        if native["counts"].is_object() {
            let total: u64 = ["passed", "failed", "errored", "skipped"]
                .into_iter()
                .map(|field| native["counts"][field].as_u64().unwrap())
                .sum();
            assert_eq!(native["tests"], total);
            assert_eq!(native["counts"]["skipped"], 1);
            let unit = json["layers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["layer"] == "unit")
                .unwrap();
            assert_eq!(unit["tests"], total);
            assert_eq!(unit["passed"], native["counts"]["passed"]);
            assert_eq!(unit["failed"], native["counts"]["failed"]);
            assert_eq!(unit["skipped"], 1);
        }
    }
}

#[test]
fn each_producer_fixture_runs_through_the_cli() {
    for xml in [
        include_str!("../../duhem-runtime/tests/fixtures/junit/nextest.xml"),
        include_str!("../../duhem-runtime/tests/fixtures/junit/pytest.xml"),
        include_str!("../../duhem-runtime/tests/fixtures/junit/jest-junit.xml"),
        include_str!("../../duhem-runtime/tests/fixtures/junit/playwright.xml"),
        include_str!("../../duhem-runtime/tests/fixtures/junit/go-junit-report.xml"),
    ] {
        let (json, _tmp) = execute(Some(xml), 0, "", false);
        assert_eq!(json["native"][0]["outcome"], "fail");
        assert_eq!(json["native"][0]["counts"]["passed"], 1);
        assert_eq!(json["native"][0]["counts"]["failed"], 1);
        assert_eq!(json["native"][0]["counts"]["skipped"], 1);
    }
}

#[tokio::test]
async fn junit_evidence_is_complete_and_messages_are_masked() {
    use duhem_evidence::{SqliteStore, Store};
    let xml = FAIL.replace("assert 1 == 2", "junit-secret-value");
    let (json, tmp) = execute(Some(&xml), 0, "", true);
    let native = &json["native"][0];
    assert_eq!(
        native["counts"],
        serde_json::json!({"passed": 1, "failed": 1, "errored": 1, "skipped": 1})
    );
    assert_eq!(native["failing_tests"].as_array().unwrap().len(), 2);
    assert!(!json.to_string().contains("junit-secret-value"));
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
        assert!(!String::from_utf8_lossy(&blob).contains("junit-secret-value"));
        assert!(String::from_utf8_lossy(&blob).contains("[redacted:token]"));
        if field == "results_blob_sha256" {
            let tests: serde_json::Value = serde_json::from_slice(&blob).unwrap();
            assert_eq!(tests.as_array().unwrap().len(), 4);
            assert_eq!(tests[0]["duration_ms"], 125);
            assert_eq!(tests[0]["file"], "tests/test_example.py");
        }
    }
}

#[test]
fn junit_failures_are_bounded_and_terminal_lists_remaining() {
    let cases = (0..12)
        .map(|i| {
            format!(
                "<testcase name='failure-{i}'><failure>{}</failure></testcase>",
                "x".repeat(3000)
            )
        })
        .collect::<String>();
    let xml = format!("<testsuite>{cases}</testsuite>");
    let (json, tmp) = execute(Some(&xml), 0, "", false);
    let native = &json["native"][0];
    assert_eq!(native["tests"], 12);
    assert_eq!(native["failing_tests"].as_array().unwrap().len(), 10);
    assert!(
        native["failing_tests"][0]["message"]
            .as_str()
            .unwrap()
            .ends_with("[truncated]")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .args(["run", "--no-live", "--db"])
        .arg(tmp.path().join("pretty.db"))
        .arg(tmp.path().join("leaf.yml"))
        .output()
        .unwrap();
    let pretty = String::from_utf8(output.stdout).unwrap();
    assert!(
        pretty.contains("0 passed, 12 failed, 0 errored, 0 skipped;"),
        "{pretty}"
    );
    assert!(pretty.contains("failure-0:"), "{pretty}");
    assert!(pretty.contains("2 more"), "{pretty}");
}
