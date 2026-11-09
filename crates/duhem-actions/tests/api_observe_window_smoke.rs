//! Browser-backed coverage for the windowed `api/observe` request
//! collector (#603 P3). Real Chromium against an in-process `axum`
//! fixture; ignored by default (needs `npx playwright install chromium`).
//!
//! Cases: out-of-order and pending responses, duplicate URLs, redirect
//! hops, aborted requests, outside-window exclusion, truncation,
//! missing/cross-session anchors, and legacy (no `window:`) outputs.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use duhem_actions::action::StepBoundary;
use duhem_actions::{
    Action, ActionCtx, ActionError, ActionResult, CheckBrowser, Click, Navigate, Observe, Outcome,
    RunBrowser,
};
use serde_yml::Value;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const PAGE: &str = r#"<!doctype html>
<html><head><title>window-fixture</title></head>
<body><main>
<script>
const get = (u) => fetch(u).then((r) => r.text());
get('/pre')
</script>
<button onclick="get('/slow'); get('/fast')">ooo</button>
<button onclick="get('/fast'); get('/fast')">dup</button>
<button onclick="get('/hop1')">redirect</button>
<button onclick="
  const c = new AbortController();
  fetch('/hang', { signal: c.signal }).catch(() => {});
  setTimeout(() => c.abort(), 100)">abort</button>
<button onclick="setTimeout(() => get('/late'), 800)">late</button>
<button onclick="for (let i = 0; i < 10; i++) get('/many?i=' + i)">many</button>
</main></body></html>"#;

fn ok() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

fn redirect(to: &'static str) -> impl IntoResponse {
    (StatusCode::FOUND, [(header::LOCATION, to)])
}

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

async fn start_fixture() -> Fixture {
    let app = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/pre", get(|| async { ok() }))
        .route("/fast", get(|| async { ok() }))
        .route("/late", get(|| async { ok() }))
        .route("/many", get(|| async { ok() }))
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                ok()
            }),
        )
        .route(
            "/hang",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                ok()
            }),
        )
        .route("/hop1", get(|| async { redirect("/hop2") }))
        .route("/hop2", get(|| async { redirect("/final") }))
        .route("/final", get(|| async { ok() }));
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

fn yaml(s: &str) -> Value {
    serde_yml::from_str(s).unwrap()
}

type Boundaries = BTreeMap<String, StepBoundary>;

async fn open() -> (RunBrowser, CheckBrowser) {
    let run = RunBrowser::launch(false)
        .await
        .expect("launch chromium (run `npx playwright install chromium`)");
    let check = run.open_check().await.unwrap();
    (run, check)
}

async fn load(check: &CheckBrowser, fx: &Fixture) -> Boundaries {
    let empty = Boundaries::new();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
        step_boundaries: &empty,
        session: None,
    };
    let start_ms = check.page.session_time_ms().await.unwrap();
    let r = Navigate
        .invoke(&ctx, &yaml(&format!("url: http://{}/", fx.addr)))
        .await
        .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    let finish_ms = check.page.session_time_ms().await.unwrap();
    Boundaries::from([(
        "load".into(),
        StepBoundary {
            session: None,
            start_ms,
            finish_ms,
        },
    )])
}

/// Click the button named `name` and record the step as `id`.
async fn click(check: &CheckBrowser, boundaries: &mut Boundaries, id: &str, name: &str) {
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 1,
        step_boundaries: boundaries,
        session: None,
    };
    let start_ms = check.page.session_time_ms().await.unwrap();
    let r = Click
        .invoke(&ctx, &yaml(&format!("{{ role: button, name: {name} }}")))
        .await
        .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    let finish_ms = check.page.session_time_ms().await.unwrap();
    boundaries.insert(
        id.into(),
        StepBoundary {
            session: None,
            start_ms,
            finish_ms,
        },
    );
}

async fn observe(
    check: &CheckBrowser,
    boundaries: &Boundaries,
    session: Option<&str>,
    with: &str,
) -> Result<ActionResult, ActionError> {
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 2,
        step_boundaries: boundaries,
        session,
    };
    Observe.invoke(&ctx, &yaml(with)).await
}

fn urls(r: &ActionResult) -> Vec<String> {
    r.outputs["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            q["url"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn out_of_order_pending_and_duplicate_requests() {
    let fx = start_fixture().await;
    let (_run, check) = open().await;
    let mut b = load(&check, &fx).await;
    click(&check, &mut b, "ooo", "ooo").await;

    // Closed before /slow responds: the request is pending, with no
    // invented status.
    let early = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:/slow$", window: { from_step: ooo, duration: 100ms } }"#,
    )
    .await
    .unwrap();
    assert_eq!(early.outputs["count"], 1);
    assert_eq!(early.outputs["requests"][0]["outcome"], "pending");
    assert!(early.outputs["requests"][0].get("status").is_none());

    // /fast answers first but /slow started first: order is by start.
    let all = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:/(slow|fast)$", window: { from_step: ooo, duration: 1s } }"#,
    )
    .await
    .unwrap();
    assert_eq!(all.outcome, Outcome::Ok);
    assert_eq!(all.outputs["count"], 2);
    assert_eq!(urls(&all), ["slow", "fast"]);
    let reqs = all.outputs["requests"].as_array().unwrap();
    assert!(reqs[0]["seq"].as_u64() < reqs[1]["seq"].as_u64());
    for r in reqs {
        assert_eq!(r["outcome"], "completed");
        assert_eq!(r["status"], 200);
    }
    assert_eq!(all.outputs["complete"], true);
    assert_eq!(all.outputs["truncated"], false);
    assert_eq!(all.outputs["window"]["edge"], "start");

    // Duplicate URLs stay distinct records with distinct ids.
    click(&check, &mut b, "dup", "dup").await;
    let dup = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:/fast$", window: { from_step: dup, duration: 500ms } }"#,
    )
    .await
    .unwrap();
    assert_eq!(dup.outputs["count"], 2);
    let reqs = dup.outputs["requests"].as_array().unwrap();
    assert_ne!(reqs[0]["seq"], reqs[1]["seq"]);
    assert_eq!(reqs[0]["url"], reqs[1]["url"]);
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn redirect_hops_get_own_ids_linked_by_redirected_from() {
    let fx = start_fixture().await;
    let (_run, check) = open().await;
    let mut b = load(&check, &fx).await;
    click(&check, &mut b, "go", "redirect").await;
    let r = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:/(hop1|hop2|final)$", window: { from_step: go, duration: 500ms } }"#,
    )
    .await
    .unwrap();
    assert_eq!(r.outputs["count"], 3);
    assert_eq!(urls(&r), ["hop1", "hop2", "final"]);
    let reqs = r.outputs["requests"].as_array().unwrap();
    assert!(reqs[0].get("redirected_from").is_none());
    assert_eq!(reqs[1]["redirected_from"], reqs[0]["seq"]);
    assert_eq!(reqs[2]["redirected_from"], reqs[1]["seq"]);
    assert_eq!(reqs[0]["status"], 302);
    assert_eq!(reqs[1]["status"], 302);
    assert_eq!(reqs[2]["status"], 200);
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn aborted_request_is_failed_with_error_and_no_status() {
    let fx = start_fixture().await;
    let (_run, check) = open().await;
    let mut b = load(&check, &fx).await;
    click(&check, &mut b, "go", "abort").await;
    let r = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:/hang$", window: { from_step: go, duration: 600ms } }"#,
    )
    .await
    .unwrap();
    assert_eq!(r.outputs["count"], 1);
    let rec = &r.outputs["requests"][0];
    assert_eq!(rec["outcome"], "failed");
    assert!(rec["error"].as_str().is_some_and(|e| !e.is_empty()));
    assert!(rec.get("status").is_none());
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn requests_outside_the_window_are_excluded() {
    let fx = start_fixture().await;
    let (_run, check) = open().await;
    let mut b = load(&check, &fx).await;
    click(&check, &mut b, "go", "late").await;

    // /pre started before the anchor; /late starts ~800ms after it.
    let before = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:/pre$", window: { from_step: go, duration: 200ms } }"#,
    )
    .await
    .unwrap();
    assert_eq!(before.outputs["count"], 0);
    assert_eq!(before.outputs["complete"], true);
    assert_eq!(before.outcome, Outcome::Ok);

    let after = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:/late$", window: { from_step: go, duration: 200ms } }"#,
    )
    .await
    .unwrap();
    assert_eq!(after.outputs["count"], 0);

    let inside = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:/late$", window: { from_step: go, duration: 1500ms } }"#,
    )
    .await
    .unwrap();
    assert_eq!(inside.outputs["count"], 1);

    // The load-edge window starts when the load step finished, so the
    // load step's own /pre request (started earlier) is excluded.
    let finish = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:/pre$", window: { from_step: load, edge: finish, duration: 200ms } }"#,
    )
    .await
    .unwrap();
    assert_eq!(finish.outputs["window"]["edge"], "finish");
    assert_eq!(finish.outputs["count"], 0);
    assert_eq!(finish.outputs["complete"], true);
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn overflow_sets_truncated_and_never_claims_complete() {
    let fx = start_fixture().await;
    let (_run, check) = open().await;
    check.page.set_request_cap_for_tests(3).await.unwrap();
    let mut b = load(&check, &fx).await;
    click(&check, &mut b, "go", "many").await;
    let r = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:.*", window: { from_step: go, duration: 500ms } }"#,
    )
    .await
    .unwrap();
    assert_eq!(r.outputs["truncated"], true);
    assert_eq!(r.outputs["complete"], false);
    assert!(r.outputs["requests"].as_array().unwrap().len() <= 3);
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn missing_or_cross_session_anchor_is_an_action_error() {
    let fx = start_fixture().await;
    let (_run, check) = open().await;
    let mut b = load(&check, &fx).await;

    // Never ran (skipped or gated steps record no boundary).
    let missing = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:.*", window: { from_step: skipped, duration: 100ms } }"#,
    )
    .await
    .unwrap_err();
    assert!(missing.to_string().contains("has not run"), "{missing}");

    // Anchor ran in another session than the observing step.
    b.insert(
        "other".into(),
        StepBoundary {
            session: Some("admin".into()),
            start_ms: 0.0,
            finish_ms: 1.0,
        },
    );
    for session in [None, Some("user")] {
        let cross = observe(
            &check,
            &b,
            session,
            r#"{ url_pattern: "re:.*", window: { from_step: other, duration: 100ms } }"#,
        )
        .await
        .unwrap_err();
        assert!(
            cross.to_string().contains("different browser session"),
            "{cross}"
        );
    }
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn legacy_mode_without_window_keeps_first_match_outputs() {
    let fx = start_fixture().await;
    let (_run, check) = open().await;
    let mut b = load(&check, &fx).await;
    click(&check, &mut b, "go", "dup").await;
    let r = observe(
        &check,
        &b,
        None,
        r#"{ url_pattern: "re:/fast$", timeout: 2s }"#,
    )
    .await
    .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    assert_eq!(r.outputs["method"], "GET");
    assert_eq!(r.outputs["status"], 200);
    assert_eq!(r.outputs["body_text"], "ok");
    for windowed in ["count", "requests", "window", "complete", "truncated"] {
        assert!(!r.outputs.contains_key(windowed), "{windowed} leaked");
    }
}
