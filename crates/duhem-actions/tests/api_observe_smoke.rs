//! End-to-end smoke for `api/observe` against an in-process `axum`
//! fixture. Drives a real Chromium via the official Playwright Node
//! sidecar (`crate::browser`; #71) and exercises the network-observation
//! channel restored in #72.
//!
//! Ignored in CI by default — running these requires
//! `npx playwright install chromium` (multi-hundred-MB download). The
//! `just test browser-actions` runs them locally.
//!
//! Cases (per spec Plan/Test on #72):
//!
//! - `observe_captures_fetch_triggered_by_click` — a `ui/click` fires a
//!   `fetch()` POST with a JSON body; observe matches it by URL+method
//!   and surfaces `status` / `body` / `body_text` / `headers` /
//!   `request_body` / `request_headers` / `method` / `url`.
//! - `observe_json_parse_failure_emits_observation` — a response that
//!   declares `application/json` but isn't parseable yields a `null`
//!   `body` plus an `api.json_parse_failure` observation.
//! - `observe_times_out_when_no_event_matches` — no matching traffic
//!   within `within:` yields `Outcome::Timeout`, promptly.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use duhem_actions::{Action, ActionCtx, Click, Navigate, Observe, Outcome, RunBrowser};
use serde_yml::Value;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const STATIC_HTML: &str = r#"<!doctype html>
<html><head><title>observe-fixture</title></head>
<body>
  <main>
    <button id="make" onclick="
      fetch('/api/projects', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'Demo' }),
      })
    ">Make Project</button>

    <button id="bad" onclick="fetch('/api/bad')">Bad JSON</button>
  </main>
</body></html>"#;

async fn create_project() -> impl IntoResponse {
    (
        StatusCode::CREATED,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"id":"p1","name":"Demo"}"#,
    )
}

async fn bad_json() -> impl IntoResponse {
    // Declares JSON but is not parseable — drives the
    // `api.json_parse_failure` observation path.
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        "{not json",
    )
}

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

async fn start_fixture() -> Fixture {
    let app = Router::new()
        .route("/", get(|| async { axum::response::Html(STATIC_HTML) }))
        .route("/api/projects", post(create_project))
        .route("/api/bad", get(bad_json));
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

fn url(fx: &Fixture) -> String {
    format!("http://{}/", fx.addr)
}

async fn fresh_browser() -> Arc<RunBrowser> {
    Arc::new(
        RunBrowser::launch(false)
            .await
            .expect("launch chromium (run `npx playwright install chromium`)"),
    )
}

fn yaml(s: &str) -> Value {
    serde_yml::from_str(s).unwrap()
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn observe_captures_fetch_triggered_by_click() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();

    // Click fires the fetch; observe (run after, per the v1 ordering
    // caveat) finds it in the page's recorded traffic.
    Click
        .invoke(&ctx, &yaml(r#"{ role: button, name: "Make Project" }"#))
        .await
        .unwrap();

    let endpoint = format!("http://{}/api/projects", fx.addr);
    let r = Observe
        .invoke(
            &ctx,
            &yaml(&format!(
                r#"
method: POST
url_pattern: "{endpoint}"
within: 2s
"#,
            )),
        )
        .await
        .unwrap();

    assert_eq!(r.outcome, Outcome::Ok);
    assert_eq!(
        r.outputs.get("method").and_then(|v| v.as_str()),
        Some("POST")
    );
    assert_eq!(
        r.outputs.get("url").and_then(|v| v.as_str()),
        Some(endpoint.as_str())
    );
    assert_eq!(r.outputs.get("status").and_then(|v| v.as_u64()), Some(201));
    // Response body parsed as JSON (Content-Type: application/json).
    assert_eq!(r.outputs["body"]["id"], serde_json::json!("p1"));
    assert_eq!(
        r.outputs.get("body_text").and_then(|v| v.as_str()),
        Some(r#"{"id":"p1","name":"Demo"}"#)
    );
    assert_eq!(
        r.outputs["headers"]["content-type"]
            .as_str()
            .map(|s| s.starts_with("application/json")),
        Some(true)
    );
    // Request side: the JSON body the page POSTed, and its headers.
    assert_eq!(r.outputs["request_body"]["name"], serde_json::json!("Demo"));
    assert_eq!(
        r.outputs["request_headers"]["content-type"]
            .as_str()
            .map(|s| s.starts_with("application/json")),
        Some(true)
    );
    assert!(r.observations.is_empty());
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn observe_json_parse_failure_emits_observation() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();

    Click
        .invoke(&ctx, &yaml(r#"{ role: button, name: "Bad JSON" }"#))
        .await
        .unwrap();

    let endpoint = format!("http://{}/api/bad", fx.addr);
    let r = Observe
        .invoke(
            &ctx,
            &yaml(&format!(
                r#"
url_pattern: "{endpoint}"
within: 2s
"#,
            )),
        )
        .await
        .unwrap();

    assert_eq!(r.outcome, Outcome::Ok);
    // Declared JSON but unparseable → body null, body_text preserved,
    // and a structured parse-failure observation.
    assert_eq!(r.outputs["body"], serde_json::Value::Null);
    assert_eq!(
        r.outputs.get("body_text").and_then(|v| v.as_str()),
        Some("{not json")
    );
    assert_eq!(r.observations.len(), 1);
    assert_eq!(r.observations[0].kind, "api.json_parse_failure");
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn observe_times_out_when_no_event_matches() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();

    // Nothing ever requests this path → timeout within `within:`.
    let started = Instant::now();
    let r = Observe
        .invoke(
            &ctx,
            &yaml(r#"{ url_pattern: "http://does.not/match", within: 300ms }"#),
        )
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(r.outcome, Outcome::Timeout);
    // Loose upper bound — verifies we honored `within: 300ms` rather
    // than the 5s default.
    assert!(
        elapsed < Duration::from_millis(2_000),
        "elapsed = {elapsed:?}"
    );
}
