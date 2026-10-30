//! Real children, HTTP, evidence, ownership, cleanup and replay for #579.
use std::collections::BTreeMap;
use std::sync::Arc;

use duhem_evidence::{EventPayload, SqliteStore, Trace, replay};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use duhem_judge::InconclusiveCause;
use duhem_judge::VerdictState;
use duhem_runtime::{Engine, RunOutcome};
use duhem_schema::VerificationDefinition;
use serde_json::{Value, json};

fn definition(scenario: &str, dir: &std::path::Path) -> VerificationDefinition {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/process.py");
    let yaml = format!("verification: Owned processes\ninputs:\n  secret: {{ type: string, secret: true }}\n{scenario}")
        .replace("@FIXTURE@", &serde_json::to_string(&fixture).unwrap())
        .replace("@STATE@", &serde_json::to_string(&dir.join("state")).unwrap());
    VerificationDefinition::from_yaml_str(&yaml).unwrap()
}

async fn run(scenario: &str) -> (RunOutcome, Trace, tempfile::TempDir) {
    run_config(scenario, serde_yml::from_str("{}").unwrap()).await
}

async fn run_config(
    scenario: &str,
    defaults: duhem_schema::ManifestDefaults,
) -> (RunOutcome, Trace, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let definition = definition(scenario, dir.path());
    let store = Arc::new(
        SqliteStore::open(dir.path().join("evidence.db"))
            .await
            .unwrap(),
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let result = Engine::new()
        .with_defaults(&defaults)
        .with_progress(tx)
        .with_store(store.clone())
        .run_with_metadata(
            &definition,
            BTreeMap::from([("secret".into(), json!("split-process-credential-579"))]),
        )
        .await
        .unwrap();
    let trace = Trace::from_store(store.as_ref(), &result.run_id)
        .await
        .unwrap();
    let mut live = Vec::new();
    while let Ok(event) = rx.try_recv() {
        live.push(event);
    }
    assert_eq!(
        serde_json::to_value(&live).unwrap(),
        serde_json::to_value(trace.events()).unwrap()
    );
    assert!(
        !serde_json::to_string(&live)
            .unwrap()
            .contains("split-process-credential-579")
    );
    if !result.cleanup.is_empty() {
        eprintln!("cleanup findings: {:?}", result.cleanup);
    }
    for event in trace.events() {
        if let EventPayload::ProcessObserved {
            operation,
            facts,
            detail: Some(detail),
            ..
        } = &event.payload
        {
            eprintln!("{operation}: {detail}");
            eprintln!("process log facts: {}", facts["logs"]);
        }
    }
    assert_eq!(replay(&trace).unwrap().run.state, result.verdict.state);
    (result, trace, dir)
}

fn process_facts(trace: &Trace, operation: &str) -> Vec<Value> {
    trace
        .events()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ProcessObserved {
                operation: op,
                facts,
                ..
            } if op == operation => Some(facts.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn real_http_readiness_restart_and_persistent_state() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let scenario = r#"
criteria:
  - id: AC-1
    description: A local server restarts with its state preserved.
    checks:
      - id: AC-1.1
        setup:
          - id: server
            uses: process/start
            with:
              command: python3
              args: [-u, @FIXTURE@, server, @STATE@, "@PORT@"]
              ready:
                http: { url: "http://127.0.0.1:@PORT@/health", status: 200, headers: { X-Ready: "yes" } }
                timeout: 5s
        steps:
          - id: before
            uses: api/call
            with: { url: "http://127.0.0.1:@PORT@/state", method: GET }
          - id: restarted
            uses: process/restart
            with: { handle: $setup.server.outputs.handle }
          - id: after
            uses: api/call
            with: { url: "http://127.0.0.1:@PORT@/state", method: GET }
          - id: status
            uses: process/inspect
            with: { handle: $setup.server.outputs.handle }
        assertions:
          - $steps.before.outputs.body_text == "1"
          - $steps.after.outputs.body_text == "2"
          - $steps.restarted.outputs.ready == true
          - $steps.restarted.outputs.generation == 2
          - $steps.status.outputs.state == "stopped"
"#.replace("@PORT@", &port.to_string());
    let (outcome, trace, dir) = run(&scenario).await;
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("state")).unwrap(),
        "2"
    );
    let starts = process_facts(&trace, "process/start");
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0]["generation"], 1);
    assert_eq!(starts[1]["generation"], 2);
    assert_eq!(process_facts(&trace, "finalizer")[0]["state"], "stopped");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn descendants_are_stopped_after_leader_exits_and_unrelated_process_survives() {
    let mut unrelated = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let (outcome, trace, _) = run(r#"
criteria:
  - id: AC-1
    description: Cleanup owns the descendant group only.
    checks:
      - id: AC-1.1
        setup:
          - id: tree
            uses: process/start
            with:
              command: python3
              args: [-u, @FIXTURE@, tree, @STATE@]
              stop_timeout: 50ms
              ready: { log: { stream: stdout, pattern: READY }, timeout: 5s }
        steps:
          - id: stopped
            uses: process/stop
            with: { handle: $setup.tree.outputs.handle }
          - id: again
            uses: process/stop
            with: { handle: $setup.tree.outputs.handle }
        assertions:
          - $steps.stopped.outputs.stopped == true
          - $steps.stopped.outputs.escalated == true
          - $steps.again.outputs.already_exited == true
"#)
    .await;
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    assert_eq!(
        process_facts(&trace, "process/stop")[0]["stop"]["escalated"],
        true
    );
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.kill().await.unwrap();
    unrelated.wait().await.unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn bounded_logs_mask_secrets_across_reads_and_keep_cursor_loss_honest() {
    let (outcome, trace, _) = run(r#"
criteria:
  - id: AC-1
    description: Log windows do not claim completeness after loss.
    checks:
      - id: AC-1.1
        setup:
          - id: child
            uses: process/start
            with:
              command: python3
              args: [-u, @FIXTURE@, split]
              env: { PROCESS_SECRET: $inputs.secret }
              ready: { log: { stream: stdout, pattern: END }, timeout: 5s }
        steps:
          - id: logs
            uses: process/inspect
            with: { handle: $setup.child.outputs.handle }
          - id: delta
            uses: process/inspect
            with: { handle: $setup.child.outputs.handle, cursor: $steps.logs.outputs.next_cursor }
        assertions:
          - $steps.logs.outputs.truncated == true
          - $steps.logs.outputs.dropped_bytes > 0
"#)
    .await;
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    let facts = process_facts(&trace, "finalizer").pop().unwrap();
    for stream in ["stdout", "stderr"] {
        let bytes: usize = facts["logs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|record| record["stream"] == stream)
            .map(|record| record["text"].as_str().unwrap().len())
            .sum();
        assert!(bytes <= 64 * 1024);
    }
    let serialized = serde_json::to_string(trace.events()).unwrap();
    assert!(!serialized.contains("split-process-credential-579"));
    assert!(!serialized.contains("process-credential-579"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn a_secret_spanning_reads_is_masked_before_retention() {
    let (outcome, trace, _) = run(r#"
criteria:
  - id: AC-1
    description: Stream fragments cannot expose a registered secret.
    checks:
      - id: AC-1.1
        setup:
          - id: child
            uses: process/start
            with:
              command: python3
              args: [-u, @FIXTURE@, secret]
              env: { PROCESS_SECRET: $inputs.secret }
              ready: { log: { stream: stdout, pattern: END }, timeout: 5s }
        steps: []
        assertions: ["true"]
"#)
    .await;
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    let serialized = serde_json::to_string(trace.events()).unwrap();
    assert!(serialized.contains("[redacted:secret]"));
    assert!(!serialized.contains("split-process-credential-579"));
    assert!(!serialized.contains("process-credential-579"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn missing_readiness_is_environment_in_setup_and_step_error_in_body() {
    for phase in ["setup", "steps"] {
        for mode in ["exit", "unready"] {
            let scenario = format!(
                r#"
criteria:
  - id: AC-1
    description: Failed readiness supplies no product verdict.
    checks:
      - id: AC-1.1
        {phase}:
          - uses: process/start
            with:
              command: python3
              args: [-u, @FIXTURE@, {mode}]
              ready: {{ log: {{ stream: stdout, pattern: NEVER }}, timeout: 150ms }}
        assertions: ["true"]
"#
            );
            let (outcome, trace, _) = run(&scenario).await;
            let expected = if phase == "setup" {
                InconclusiveCause::EnvironmentError
            } else {
                InconclusiveCause::MissingObservation
            };
            assert_eq!(outcome.verdict.state, VerdictState::Inconclusive(expected));
            assert_eq!(process_facts(&trace, "finalizer")[0]["state"], "stopped");
            assert!(trace.events().iter().any(|event| matches!(
                &event.payload,
                EventPayload::StepFinished {
                    detail: Some(_),
                    ..
                } | EventPayload::SetupStepFinished {
                    detail: Some(_),
                    ..
                }
            )));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn outer_handles_are_inspect_only_and_body_failure_still_finalizes() {
    let (outcome, trace, _) = run(r#"
setup:
  - id: shared
    uses: process/start
    with: { command: python3, args: [-u, @FIXTURE@, idle] }
criteria:
  - id: AC-1
    description: Siblings cannot mutate the shared environment.
    checks:
      - id: AC-1.1
        steps:
          - id: shared
            uses: process/inspect
            with: { handle: $setup.shared.outputs.handle }
          - uses: process/restart
            with: { handle: $setup.shared.outputs.handle }
        assertions: ["true"]
      - id: AC-1.2
        steps:
          - id: shared
            uses: process/inspect
            with: { handle: $setup.shared.outputs.handle }
        assertions: ["$steps.shared.outputs.running == true"]
"#)
    .await;
    assert!(matches!(
        outcome.verdict.state,
        VerdictState::Inconclusive(_)
    ));
    assert_eq!(
        outcome.verdict.criteria[0].checks[1].state,
        VerdictState::Pass
    );
    assert_eq!(process_facts(&trace, "process/start")[0]["ready"], false);
    assert_eq!(process_facts(&trace, "finalizer").len(), 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn diagnostic_fixture_still_has_required_process_finalizer() {
    let (outcome, trace, _) = run(r#"
fixtures:
  watcher:
    cleanup_policy: diagnostic_only
    up:
      - id: child
        uses: process/start
        with: { command: python3, args: [-u, @FIXTURE@, idle] }
    down:
      - uses: process/inspect
        with: { handle: $fixture.watcher.child.outputs.handle }
criteria:
  - id: AC-1
    description: A diagnostic fixture cannot weaken owned cleanup.
    checks:
      - id: AC-1.1
        needs: [watcher]
        steps: []
        assertions: ["false"]
"#)
    .await;
    assert_eq!(outcome.verdict.state, VerdictState::Fail);
    assert!(trace.events().iter().any(|event| matches!(&event.payload,
        EventPayload::SetupStarted { cleanup_policy: Some(policy), fixture_name: Some(name), .. }
        if policy == "required" && name == "watcher")));
    assert_eq!(process_facts(&trace, "finalizer")[0]["state"], "stopped");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn retries_get_fresh_processes_after_previous_attempt_finalizes() {
    let defaults = serde_yml::from_str("timeout: 5s\nretry: { max: 1, backoff: linear }").unwrap();
    let (outcome, trace, _) = run_config(
        r#"
criteria:
  - id: AC-1
    description: Each attempt owns a fresh environment.
    checks:
      - id: AC-1.1
        setup:
          - id: child
            uses: process/start
            with: { command: python3, args: [-u, @FIXTURE@, idle] }
        steps:
          - uses: cli/invoke
            with: { command: [sleep, "1"], timeout: 1ms }
        assertions: ["true"]
"#,
        defaults,
    )
    .await;
    assert_eq!(
        outcome.verdict.state,
        VerdictState::Inconclusive(InconclusiveCause::Timeout)
    );
    let mut handles = Vec::new();
    let mut attempts = Vec::new();
    let mut finalized = true;
    for event in trace.events() {
        if let EventPayload::ProcessObserved {
            operation,
            handle,
            owner,
            facts,
            ..
        } = &event.payload
        {
            if operation == "process/start" {
                assert!(finalized, "attempt started before previous owned cleanup");
                handles.push(handle.clone());
                attempts.push(owner.attempt.clone());
                finalized = false;
            } else if operation == "finalizer" {
                assert_eq!(facts["state"], "stopped");
                finalized = true;
            }
        }
    }
    assert_eq!(handles.len(), 2);
    assert_ne!(handles[0], handles[1]);
    assert_ne!(attempts[0], attempts[1]);
    assert!(finalized);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn sibling_run_and_retired_generation_handles_are_rejected() {
    let (outcome, trace, dir) = run(r#"
criteria:
  - id: AC-1
    description: Process handles carry run, owner and generation identity.
    checks:
      - id: AC-1.1
        steps:
          - id: child
            uses: process/start
            with: { command: python3, args: [-u, @FIXTURE@, idle] }
          - uses: cli/invoke
            with:
              command: [python3, -c, "from pathlib import Path; import sys; Path(sys.argv[1]).write_text(sys.argv[2])", @STATE@, $steps.child.outputs.handle]
          - uses: process/restart
            with: { handle: $steps.child.outputs.handle }
          - uses: process/restart
            with: { handle: $steps.child.outputs.handle }
        assertions: ["true"]
      - id: AC-1.2
        steps:
          - id: loaded
            uses: cli/invoke
            with:
              command: [python3, -c, "from pathlib import Path; import sys; sys.stdout.write(Path(sys.argv[1]).read_text())", @STATE@]
          - uses: process/stop
            with: { handle: $steps.loaded.outputs.stdout }
        assertions: ["true"]
"#).await;
    assert!(matches!(
        outcome.verdict.state,
        VerdictState::Inconclusive(_)
    ));
    assert_eq!(process_facts(&trace, "process/start").len(), 2);
    let details: Vec<_> = trace
        .events()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::StepFinished {
                detail: Some(detail),
                ..
            } => Some(detail.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("already been restarted"))
    );
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("sibling check or another retry"))
    );

    let handle = std::fs::read_to_string(dir.path().join("state")).unwrap();
    let next = definition(
        r#"
criteria:
  - id: AC-1
    description: A different run cannot use the old handle.
    checks:
      - id: AC-1.1
        steps:
          - uses: process/stop
            with: { handle: $inputs.secret }
        assertions: ["true"]
"#,
        dir.path(),
    );
    let store = Arc::new(
        SqliteStore::open(dir.path().join("other.db"))
            .await
            .unwrap(),
    );
    let result = Engine::new()
        .with_store(store.clone())
        .run_with_metadata(&next, BTreeMap::from([("secret".into(), json!(handle))]))
        .await
        .unwrap();
    let trace = Trace::from_store(store.as_ref(), &result.run_id)
        .await
        .unwrap();
    assert!(matches!(
        result.verdict.state,
        VerdictState::Inconclusive(_)
    ));
    assert!(trace.events().iter().any(|event| matches!(&event.payload,
        EventPayload::StepFinished { detail: Some(detail), .. } if detail.contains("unknown process handle"))));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn required_finalizer_failure_preserves_body_and_prevents_restart() {
    struct EscapedFixture(std::path::PathBuf);
    impl Drop for EscapedFixture {
        fn drop(&mut self) {
            if let Ok(raw) = std::fs::read_to_string(&self.0)
                && let Ok(pid) = raw.parse::<i32>()
            {
                // This PID was authored by our deliberately escaped fixture,
                // never obtained from the user's process-action inputs.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }
    for restart in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let _escaped = EscapedFixture(dir.path().join("state"));
        let body = if restart {
            "steps:\n          - uses: process/restart\n            with: { handle: $fixture.watcher.child.outputs.handle }"
        } else {
            "steps: []"
        };
        let scenario = format!(
            r#"
fixtures:
  watcher:
    cleanup_policy: diagnostic_only
    up:
      - id: child
        uses: process/start
        with:
          command: python3
          args: [-u, @FIXTURE@, escaped-pipe, @STATE@]
          kill_timeout: 100ms
          ready: {{ log: {{ stream: stdout, pattern: READY }}, timeout: 5s }}
    down:
      - uses: process/inspect
        with: {{ handle: $fixture.watcher.child.outputs.handle }}
criteria:
  - id: AC-1
    description: Incomplete owned cleanup independently blocks the gate.
    checks:
      - id: AC-1.1
        needs: [watcher]
        {body}
        assertions: ["true"]
"#
        );
        let definition = definition(&scenario, dir.path());
        let store = Arc::new(
            SqliteStore::open(dir.path().join("evidence.db"))
                .await
                .unwrap(),
        );
        let result = Engine::new()
            .with_store(store.clone())
            .run_with_metadata(
                &definition,
                BTreeMap::from([("secret".into(), json!("unused-secret-579"))]),
            )
            .await
            .unwrap();
        let trace = Trace::from_store(store.as_ref(), &result.run_id)
            .await
            .unwrap();
        assert_eq!(
            result.verdict.state,
            VerdictState::Fail,
            "{:?}",
            result.cleanup
        );
        assert_eq!(
            result.body_verdict,
            Some(if restart {
                VerdictState::Inconclusive(InconclusiveCause::MissingObservation)
            } else {
                VerdictState::Pass
            })
        );
        assert_eq!(result.cleanup.len(), 1);
        assert!(result.cleanup[0].required);
        assert!(
            result.cleanup[0]
                .detail
                .as_ref()
                .unwrap()
                .contains("log pipes did not close")
        );
        assert_eq!(
            process_facts(&trace, "process/start").len(),
            1,
            "failed stop spawned a replacement"
        );
        assert_eq!(replay(&trace).unwrap().run.state, VerdictState::Fail);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test(flavor = "multi_thread")]
async fn cancellation_drains_authored_cleanup_and_records_required_finalizer() {
    const CHILD: &str = "DUHEM_PROCESS_CANCELLATION_CHILD";
    if let Ok(root) = std::env::var(CHILD) {
        let root = std::path::PathBuf::from(root);
        let yaml = r#"
criteria:
  - id: AC-1
    description: Cancellation retains owned cleanup facts.
    checks:
      - id: AC-1.1
        setup:
          - uses: process/start
            with:
              command: python3
              args: [-u, @FIXTURE@, idle]
              ready: { log: { stream: stdout, pattern: READY }, timeout: 5s }
        steps:
          - uses: cli/invoke
            with:
              command: [python3, -c, "from pathlib import Path; import sys,time; Path(sys.argv[1]).write_text('ready'); time.sleep(30)", @READY@]
              timeout: 35s
        assertions: ["true"]
        teardown:
          - uses: cli/invoke
            with:
              command: [python3, -c, "from pathlib import Path; import sys; Path(sys.argv[1]).write_text('cleaned')", @CLEANED@]
"#.replace("@READY@", &serde_json::to_string(&root.join("ready")).unwrap())
            .replace("@CLEANED@", &serde_json::to_string(&root.join("cleaned")).unwrap());
        let definition = definition(&yaml, &root);
        let store = Arc::new(SqliteStore::open(root.join("evidence.db")).await.unwrap());
        let result = Engine::new()
            .with_store(store)
            .with_run_id("process-cancel")
            .run_with_metadata(
                &definition,
                BTreeMap::from([("secret".into(), json!("unused-secret-579"))]),
            )
            .await;
        assert!(matches!(
            result,
            Err(duhem_runtime::EngineError::Aborted { .. })
        ));
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cancellation_drains_authored_cleanup_and_records_required_finalizer",
            "--nocapture",
        ])
        .env(CHILD, dir.path())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    while !dir.path().join("ready").exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "child exited before ready"
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "child never reached body"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    unsafe {
        assert_eq!(libc::kill(child.id().unwrap() as i32, libc::SIGTERM), 0);
    }
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    assert!(dir.path().join("cleaned").exists());
    let store = SqliteStore::open(dir.path().join("evidence.db"))
        .await
        .unwrap();
    let trace = Trace::from_store(&store, "process-cancel").await.unwrap();
    assert!(matches!(
        trace.events().last().unwrap().payload,
        EventPayload::RunAborted { .. }
    ));
    assert_eq!(process_facts(&trace, "finalizer")[0]["state"], "stopped");
    for event in trace.events() {
        if let EventPayload::ProcessObserved { pid, operation, .. } = &event.payload
            && operation == "process/start"
        {
            assert_eq!(
                unsafe { libc::kill(*pid as i32, 0) },
                -1,
                "owned child survived cancellation"
            );
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[tokio::test]
async fn unsupported_platform_rejects_before_spawn() {
    let (outcome, trace, dir) = run(r#"
criteria:
  - id: AC-1
    description: Unsupported platforms spawn nothing.
    checks:
      - id: AC-1.1
        steps:
          - uses: process/start
            with: { command: python3, args: [-u, @FIXTURE@, server, @STATE@, "12345"] }
        assertions: ["true"]
"#)
    .await;
    assert!(matches!(
        outcome.verdict.state,
        VerdictState::Inconclusive(_)
    ));
    assert!(!dir.path().join("state").exists());
    assert!(process_facts(&trace, "process/start").is_empty());
    assert!(trace.events().iter().any(|event| matches!(&event.payload,
        EventPayload::StepFinished { detail: Some(detail), .. } if detail.contains("no process was spawned"))));
}
