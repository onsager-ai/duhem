use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use duhem_actions::action::StepBoundary;
use duhem_actions::{
    Action, ActionCtx, ActionError, ActionResult, CheckBrowser, Click, Navigate, ObserveConsole,
    Outcome, RunBrowser,
};
use serde_yml::Value;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const PAGE: &str = r#"<!doctype html><html><body>
<button onclick="console.log('one'); console.warn('two'); console.error('three'); setTimeout(() => { throw new Error('uncaught-control') }, 10)">emit</button>
<button onclick="setTimeout(() => console.log('late'), 800)">late</button>
<button onclick="console.log('a'); console.log('b'); console.log('c')">many</button>
</body></html>"#;

fn ok() -> impl IntoResponse {
    (StatusCode::OK, [(header::CACHE_CONTROL, "no-store")], "ok")
}

fn redirect(to: &'static str) -> impl IntoResponse {
    (
        StatusCode::FOUND,
        [(header::LOCATION, to), (header::CACHE_CONTROL, "no-store")],
    )
}

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

async fn start_fixture() -> Fixture {
    let app = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/favicon.ico", get(|| async { "" }))
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
    ObserveConsole.invoke(&ctx, &yaml(with)).await
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn records_distinct_events_and_excludes_outside_window() {
    let fx = start_fixture().await;
    let (_run, check) = open().await;
    let mut b = load(&check, &fx).await;
    let clean = observe(
        &check,
        &b,
        None,
        "window: { from_step: load, duration: 200ms }",
    )
    .await
    .unwrap();
    assert_eq!(clean.outputs["complete"], true);
    assert_eq!(clean.outputs["page_error_count"], 0);
    assert_eq!(clean.outputs["console_count"], 0);
    click(&check, &mut b, "emit", "emit").await;
    let r = observe(
        &check,
        &b,
        None,
        "window: { from_step: emit, duration: 500ms }",
    )
    .await
    .unwrap();
    assert_eq!(r.outputs["page_error_count"], 1);
    assert_eq!(r.outputs["console_count"], 3);
    assert_eq!(r.outputs["console_error_count"], 1);
    assert_eq!(r.outputs["console"][0]["level"], "log");
    assert_eq!(r.outputs["console"][1]["level"], "warning");
    assert_eq!(r.outputs["console"][2]["level"], "error");
    assert!(
        r.outputs["page_errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("uncaught-control")
    );
    click(&check, &mut b, "late", "late").await;
    let late = observe(
        &check,
        &b,
        None,
        "window: { from_step: late, duration: 200ms }",
    )
    .await
    .unwrap();
    assert_eq!(late.outputs["console_count"], 0);
    assert_eq!(late.outputs["complete"], true);
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn truncated_and_missing_or_cross_session_anchors() {
    let fx = start_fixture().await;
    let (_run, check) = open().await;
    check.page.set_console_cap_for_tests(1).await.unwrap();
    let mut b = load(&check, &fx).await;
    click(&check, &mut b, "many", "many").await;
    let r = observe(
        &check,
        &b,
        None,
        "window: { from_step: many, duration: 500ms }",
    )
    .await
    .unwrap();
    assert_eq!(r.outputs["truncated"], true);
    assert_eq!(r.outputs["complete"], false);
    let missing = observe(
        &check,
        &b,
        None,
        "window: { from_step: skipped, duration: 1s }",
    )
    .await
    .unwrap_err();
    assert!(missing.to_string().contains("has not run"));
    b.get_mut("many").unwrap().session = Some("other".into());
    let cross = observe(
        &check,
        &b,
        None,
        "window: { from_step: many, duration: 1s }",
    )
    .await
    .unwrap_err();
    assert!(cross.to_string().contains("different browser session"));
}
