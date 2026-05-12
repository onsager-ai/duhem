//! End-to-end integration test for the v1 step executor.
//!
//! Drives `Engine::run` against the static-page fixture from
//! `duhem-actions` (issue #12) via a real Playwright browser
//! against an in-process axum fixture. Per the spec on issue #15
//! this is the worked-example check — same fixture, same
//! assertions, executed through the full pipeline.
//!
//! `#[ignore]`'d by default because Playwright's chromium binary
//! has to be installed once via `npx playwright install chromium`.
//! `just test-runtime-smoke` (or `cargo test -p duhem-runtime --
//! --ignored`) runs it locally.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Router;
use axum::routing::get;
use duhem_actions::RunBrowser;
use duhem_evidence::{EventPayload, Trace, replay};
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const STATIC_HTML: &str = r#"<!doctype html>
<html><head><title>fixture</title></head>
<body>
  <main>
    <button id="create" onclick="
      var b = document.createElement('div');
      b.setAttribute('role', 'alert');
      b.textContent = 'Created';
      document.body.appendChild(b);
    ">Create</button>
  </main>
</body></html>"#;

const FIXTURE_YAML: &str = include_str!("../../duhem-actions/tests/fixtures/static-page.yml");

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

async fn start_fixture() -> Fixture {
    let app = Router::new().route("/", get(|| async { axum::response::Html(STATIC_HTML) }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Fixture {
        addr,
        _server: server,
    }
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn static_page_fixture_passes_end_to_end_and_replays() {
    let fx = start_fixture().await;
    let url = format!("http://{}/", fx.addr);

    let def = VerificationDefinition::from_yaml_str(FIXTURE_YAML).expect("parse fixture");

    let browser = RunBrowser::launch(false)
        .await
        .expect("launch chromium (run `npx playwright install chromium`)");

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut engine = Engine::new()
        .with_browser(browser)
        .with_evidence_root(tmp.path());

    let mut inputs = BTreeMap::new();
    inputs.insert("fixture_url".to_string(), url);

    let verdict = engine.run(&def, inputs).await.expect("engine.run");
    assert_eq!(verdict.state, VerdictState::Pass, "verdict = {verdict:?}");

    // The trace under `tmp` should replay to the same verdict —
    // that's the §11.2 reproducibility commitment exercised through
    // the whole pipeline.
    let entries: Vec<_> = std::fs::read_dir(tmp.path())
        .expect("readdir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    assert_eq!(entries.len(), 1, "exactly one run directory");
    let trace = Trace::open(&entries[0]).expect("open trace");

    // Setup ran once (issue #20). Evidence carries the boundary
    // markers and at least one `setup_step_finished` event with
    // outcome `Ok`. The check still passes thanks to the per-check
    // assertion of `$setup.probe.outputs.satisfied`.
    let events = trace.events();
    let saw_setup_started = events
        .iter()
        .any(|e| matches!(e.payload, EventPayload::SetupStarted { .. }));
    let saw_setup_finished_ok = events
        .iter()
        .any(|e| matches!(e.payload, EventPayload::SetupFinished { aborted: false },));
    assert!(saw_setup_started, "expected setup_started event");
    assert!(
        saw_setup_finished_ok,
        "expected setup_finished aborted=false"
    );

    let replayed = replay(&trace).expect("replay");
    assert_eq!(replayed.run.state, VerdictState::Pass);
}
