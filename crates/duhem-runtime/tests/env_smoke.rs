//! End-to-end smoke for `environment:` lifecycle.
//!
//! Drives `Engine::run` against a VD with `environment.up:` /
//! `down:` / `ready:` while a real `axum` server stands in for the
//! SUT. Covers the happy path (up exits 0 → ready 200 → criteria
//! pass → down exits 0) and the readiness-success path with a
//! probe.
//!
//! Not `#[ignore]`'d — `environment:` plumbing doesn't need a
//! Playwright browser, and the VD here declares no `Step.uses`, so
//! the engine never tries to launch one. This test runs in standard
//! `cargo test` without `npx playwright install`.

use std::collections::BTreeMap;
use std::fs::Permissions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use duhem_evidence::{EventPayload, Trace};
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use tokio::net::TcpListener;

async fn start_health_server() -> std::net::SocketAddr {
    let app = Router::new().route("/healthz", get(|| async { (StatusCode::OK, "ok") }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

fn write_script(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn environment_lifecycle_runs_up_ready_down_end_to_end() {
    let addr = start_health_server().await;
    let url = format!("http://{addr}/healthz");

    let tmp = tempfile::tempdir().unwrap();
    let scripts = tmp.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let up = scripts.join("up.sh");
    let down = scripts.join("down.sh");
    write_script(&up, "#!/bin/sh\necho up-ran\nexit 0\n");
    write_script(&down, "#!/bin/sh\necho down-ran\nexit 0\n");

    let vd_path = tmp.path().join("vd.yml");
    let mut f = std::fs::File::create(&vd_path).unwrap();
    writeln!(
        f,
        r#"
verification: env-smoke
environment:
  up: ./scripts/up.sh
  down: ./scripts/down.sh
  ready:
    http:
      url: {url}
      timeout: 10s
criteria:
  - id: AC-1
    description: env up/ready/down sequence
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#
    )
    .unwrap();

    let def = VerificationDefinition::from_yaml_str(&std::fs::read_to_string(&vd_path).unwrap())
        .expect("parse");

    let store = std::sync::Arc::new(
        duhem_evidence::SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .expect("open store"),
    );
    let mut engine = Engine::new()
        .with_store(store.clone())
        .with_definition_path(vd_path.display().to_string());

    let outcome = engine
        .run_with_metadata(&def, BTreeMap::new())
        .await
        .expect("engine run");
    assert_eq!(outcome.verdict.state, VerdictState::Pass);

    let events = Trace::from_store(store.as_ref(), &outcome.run_id)
        .await
        .unwrap()
        .into_events();

    // Locate the canonical sequence of Env*/RunFinished events and
    // assert document order.
    let mut up_started = None;
    let mut up_finished = None;
    let mut ready = None;
    let mut down_started = None;
    let mut down_finished = None;
    let mut run_finished = None;
    for (i, evt) in events.iter().enumerate() {
        match &evt.payload {
            EventPayload::EnvUpStarted { .. } => up_started = Some(i),
            EventPayload::EnvUpFinished { exit_code, .. } => {
                assert_eq!(*exit_code, 0, "up: should exit 0");
                up_finished = Some(i);
            }
            EventPayload::EnvReady { ok, probe_kind, .. } => {
                assert!(ok, "readiness probe should observe 200");
                assert_eq!(probe_kind, "http");
                ready = Some(i);
            }
            EventPayload::EnvDownStarted { .. } => down_started = Some(i),
            EventPayload::EnvDownFinished { exit_code, .. } => {
                assert_eq!(*exit_code, 0, "down: should exit 0");
                down_finished = Some(i);
            }
            EventPayload::RunFinished { .. } => run_finished = Some(i),
            _ => {}
        }
    }
    let up_started = up_started.expect("EnvUpStarted present");
    let up_finished = up_finished.expect("EnvUpFinished present");
    let ready = ready.expect("EnvReady present");
    let down_started = down_started.expect("EnvDownStarted present");
    let down_finished = down_finished.expect("EnvDownFinished present");
    let run_finished = run_finished.expect("RunFinished present");
    assert!(up_started < up_finished);
    assert!(up_finished < ready);
    assert!(ready < down_started);
    assert!(down_started < down_finished);
    assert!(down_finished < run_finished);
}

#[tokio::test]
async fn environment_url_resolves_inputs_template() {
    // The `url:` field accepts a single whole-string `$inputs.<name>`
    // reference. Confirms the runtime resolution path used by the
    // worked example, where the operator parameterizes `base_url`
    // per-environment.
    let addr = start_health_server().await;
    let url = format!("http://{addr}/healthz");

    let tmp = tempfile::tempdir().unwrap();
    let scripts = tmp.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let up = scripts.join("up.sh");
    write_script(&up, "#!/bin/sh\nexit 0\n");

    let vd_path = tmp.path().join("vd.yml");
    let mut f = std::fs::File::create(&vd_path).unwrap();
    writeln!(
        f,
        r#"
verification: env-templated-url
inputs:
  health_url: {{ type: string }}
environment:
  up: ./scripts/up.sh
  ready:
    http:
      url: $inputs.health_url
      timeout: 10s
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#
    )
    .unwrap();

    let def = VerificationDefinition::from_yaml_str(&std::fs::read_to_string(&vd_path).unwrap())
        .expect("parse");
    let store = std::sync::Arc::new(
        duhem_evidence::SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .expect("open store"),
    );
    let mut engine = Engine::new()
        .with_store(store.clone())
        .with_definition_path(vd_path.display().to_string());

    let mut inputs = BTreeMap::new();
    inputs.insert("health_url".to_string(), serde_json::json!(url));
    let outcome = engine.run_with_metadata(&def, inputs).await.unwrap();
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
}
