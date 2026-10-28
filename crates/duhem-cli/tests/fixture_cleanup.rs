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

fn run_json(path: &Path, db: &Path) -> (bool, serde_json::Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .args(["run", "--reporter", "json", "--no-live", "--db"])
        .arg(db)
        .arg(path)
        .output()
        .unwrap();
    let summary = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid report: {error}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.success(), summary)
}

#[tokio::test]
async fn body_gate_cleanup_and_store_agree_for_leaf_and_manifest() {
    use duhem_evidence::{SqliteStore, Store};
    for body in ["true", "false", "inconclusive"] {
        for policy in ["required", "diagnostic_only"] {
            let tmp = tempfile::tempdir().unwrap();
            let leaf = tmp.path().join("case.yml");
            let body_steps = if body == "inconclusive" {
                "        steps: [{uses: cli/invoke, with: {command: [duhem-intentionally-nonexistent-body-command]}}]"
            } else {
                ""
            };
            let assertion = if body == "inconclusive" { "true" } else { body };
            std::fs::write(
                &leaf,
                format!(
                    r#"
verification: Cleanup gate
fixtures:
  resource:
    cleanup_policy: {policy}
    up: [{{uses: cli/invoke, with: {{command: [sh, -c, "true"]}}}}]
    down:
      - uses: cli/invoke
        with: {{command: [duhem-intentionally-nonexistent-cleanup-command]}}
criteria:
  - id: AC
    description: Body judgment stays visible
    checks:
      - id: body
        needs: [resource]
{body_steps}
        assertions: ["{assertion}"]
      - id: independent
        assertions: ["true"]
"#
                ),
            )
            .unwrap();
            let manifest = tmp.path().join("duhem.yml");
            std::fs::write(
                &manifest,
                "manifest_version: 1\nverifications:\n  - path: case.yml\n",
            )
            .unwrap();
            for path in [&leaf, &manifest] {
                let db = tmp
                    .path()
                    .join(if path == &leaf { "leaf.db" } else { "suite.db" });
                let (success, report) = run_json(path, &db);
                let summary = report.get("runs").map(|runs| &runs[0]).unwrap_or(&report);
                let body_state = summary["criteria"][0]["verdict"].as_str().unwrap();
                let gate = if policy == "required" {
                    "fail"
                } else {
                    body_state
                };
                assert_eq!(summary["verdict"], gate);
                assert_eq!(success, gate == "pass");
                assert_eq!(summary["totals"]["total"], 2);
                assert_eq!(summary["cleanup"][0]["required"], policy == "required");
                assert_eq!(summary["lifecycle"][1]["cleanup_policy"], policy);
                if gate != body_state {
                    assert_eq!(summary["body_verdict"], body_state);
                } else {
                    assert!(summary.get("body_verdict").is_none());
                }
                let store = SqliteStore::open(&db).await.unwrap();
                let record = store
                    .get_run(summary["run_id"].as_str().unwrap())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(record.verdict.unwrap().to_string(), gate);
            }
        }
    }
}

#[test]
fn real_api_poll_judges_cleanup_but_call_status_and_cli_exit_are_observations() {
    use std::io::{Read, Write};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stopped = Arc::new(AtomicBool::new(false));
    let server_stopped = stopped.clone();
    let server = std::thread::spawn(move || {
        while !server_stopped.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0; 2048];
                    let _ = stream.read(&mut request);
                    let _ = stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(1))
                }
                Err(error) => panic!("accept: {error}"),
            }
        }
    });
    for (action, with, passes) in [
        (
            "api/poll",
            format!(
                "{{method: GET, url: 'http://{address}/', until: {{status: 200}}, timeout: 50ms, interval: 5ms}}"
            ),
            false,
        ),
        (
            "api/call",
            format!("{{url: 'http://{address}/', method: GET}}"),
            true,
        ),
        (
            "api/poll",
            format!(
                "{{method: GET, url: 'http://{address}/', until: {{status: 500}}, timeout: 50ms, interval: 5ms}}"
            ),
            true,
        ),
        (
            "cli/invoke",
            "{command: [sh, -c, 'exit 17']}".to_string(),
            true,
        ),
        (
            "cli/invoke",
            "{command: [sh, -c, 'true']}".to_string(),
            true,
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let leaf = tmp.path().join("case.yml");
        std::fs::write(
            &leaf,
            format!(
                r#"
verification: Cleanup action
fixtures:
  resource:
    up: [{{uses: cli/invoke, with: {{command: [sh, -c, "true"]}}}}]
    down:
      - uses: {action}
        with: {with}
criteria:
  - id: AC
    description: Only judging actions judge
    checks: [{{id: body, needs: [resource], assertions: ["true"]}}]
"#
            ),
        )
        .unwrap();
        let (success, summary) = run_json(&leaf, &tmp.path().join("store.db"));
        assert_eq!(success, passes, "{summary}");
        if !passes {
            assert_eq!(summary["body_verdict"], "pass");
            assert_eq!(summary["lifecycle"][1]["status"], "failed");
            assert_eq!(summary["lifecycle"][1]["steps"][0]["outcome"], "ok");
            assert_eq!(summary["lifecycle"][1]["steps"][0]["judgment"], "fail");
        } else {
            assert!(summary.get("cleanup").is_none());
            assert_eq!(summary["lifecycle"][1]["status"], "passed");
        }
    }
    stopped.store(true, Ordering::SeqCst);
    server.join().unwrap();
}
