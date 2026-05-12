//! End-to-end smoke for `api/call` against an in-process `axum`
//! server.
//!
//! Drives the full pipeline — parser → engine → `api/call` →
//! evaluator → judge → evidence — through the worked-example
//! fixture at `crates/duhem-actions/tests/fixtures/api-echo.yml`
//! (spec on issue #21). The engine still launches a Playwright
//! browser per check, per that spec ("the check still opens a
//! `CheckBrowser` but `api/call` never touches it"); browser-stripping
//! for API-only Verification Definitions is a follow-up optimization.
//!
//! The spec plan item placed this test under
//! `crates/duhem-actions/tests/`, but it depends on `duhem-runtime`'s
//! `Engine::run` — putting it there would require `duhem-runtime` as
//! a dev-dependency of `duhem-actions`, creating a build-graph cycle.
//! The `duhem-runtime/tests/` placement mirrors the existing
//! `engine_smoke.rs` precedent and keeps the crate boundaries clean.
//!
//! `#[ignore]`'d in CI for the same reason as `engine_smoke.rs`: the
//! per-check browser launch needs `npx playwright install chromium`.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::{get, post};
use duhem_actions::RunBrowser;
use duhem_evidence::{Trace, replay};
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const FIXTURE_YAML: &str = include_str!("../../duhem-actions/tests/fixtures/api-echo.yml");
const FIXED_UUID: &str = "550e8400-e29b-41d4-a716-446655440000";

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

async fn start_fixture() -> Fixture {
    let app = Router::new()
        .route(
            "/echo",
            post(
                |headers: axum::http::HeaderMap, body: axum::body::Bytes| async move {
                    let ct = headers
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("text/plain")
                        .to_string();
                    (
                        StatusCode::OK,
                        [(axum::http::header::CONTENT_TYPE, ct)],
                        body,
                    )
                },
            ),
        )
        .route(
            "/uuid",
            get(|| async {
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/plain")],
                    FIXED_UUID,
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
async fn api_echo_fixture_passes_end_to_end_and_replays() {
    let fx = start_fixture().await;
    let echo_url = format!("http://{}/echo", fx.addr);
    let uuid_url = format!("http://{}/uuid", fx.addr);

    let def = VerificationDefinition::from_yaml_str(FIXTURE_YAML).expect("parse fixture");

    let browser = RunBrowser::launch(false)
        .await
        .expect("launch chromium (run `npx playwright install chromium`)");

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut engine = Engine::new()
        .with_browser(browser)
        .with_evidence_root(tmp.path());

    let mut inputs = BTreeMap::new();
    inputs.insert(
        "echo_url".to_string(),
        serde_json::Value::String(echo_url),
    );
    inputs.insert(
        "uuid_url".to_string(),
        serde_json::Value::String(uuid_url),
    );

    let verdict = engine.run(&def, inputs).await.expect("engine.run");
    assert_eq!(verdict.state, VerdictState::Pass, "verdict = {verdict:?}");

    let entries: Vec<_> = std::fs::read_dir(tmp.path())
        .expect("readdir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    assert_eq!(entries.len(), 1, "exactly one run directory");
    let trace = Trace::open(&entries[0]).expect("open trace");
    let replayed = replay(&trace).expect("replay");
    assert_eq!(replayed.run.state, VerdictState::Pass);
}
