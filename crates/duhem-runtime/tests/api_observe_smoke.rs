//! End-to-end smoke for `api/observe` against an in-process axum
//! server. Spec on issue #38.
//!
//! The fixture serves a tiny HTML page with a button that schedules a
//! `fetch('/projects', { method: 'POST', body: '{"name":"Acme"}' })`
//! 200 ms after click. The Verification Definition lives at
//! `crates/duhem-actions/tests/fixtures/api-observe.yml`; the engine
//! drives it: `ui/navigate` → `ui/click` → `api/observe`. With the v1
//! blocking listener, the delayed fetch fires while `api/observe` is
//! awaiting events, so the capture succeeds.
//!
//! `#[ignore]`'d for the same reason as `api_smoke.rs`: needs
//! `npx playwright install chromium`.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Router;
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use duhem_actions::RunBrowser;
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const FIXTURE_YAML: &str = include_str!("../../duhem-actions/tests/fixtures/api-observe.yml");

const HTML: &str = r#"<!doctype html>
<html><body>
<button id="t">trigger</button>
<script>
document.getElementById('t').addEventListener('click', () => {
  // 200ms delay so api/observe has time to subscribe to the page's
  // event stream before this fetch fires. v1 listener attaches at
  // observe-step runtime, not at check start.
  setTimeout(() => {
    fetch('/projects', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'Acme' }),
    });
  }, 200);
});
</script>
</body></html>
"#;

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

async fn start_fixture() -> Fixture {
    let app = Router::new()
        .route("/", get(|| async { Html(HTML) }))
        .route(
            "/projects",
            post(|| async {
                (
                    StatusCode::CREATED,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    r#"{"id":"01h000000000000000000000"}"#,
                )
            }),
        );
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
async fn api_observe_captures_clicked_post_and_passes_end_to_end() {
    let fx = start_fixture().await;
    let base_url = format!("http://{}/", fx.addr);

    let def = VerificationDefinition::from_yaml_str(FIXTURE_YAML).expect("parse fixture");

    let browser = RunBrowser::launch(false)
        .await
        .expect("launch chromium (run `npx playwright install chromium`)");

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut engine = Engine::new()
        .with_browser(browser)
        .with_evidence_root(tmp.path());

    let mut inputs = BTreeMap::new();
    inputs.insert("base_url".to_string(), serde_json::Value::String(base_url));

    let verdict = engine.run(&def, inputs).await.expect("engine.run");
    assert_eq!(verdict.state, VerdictState::Pass, "verdict = {verdict:?}");
}
