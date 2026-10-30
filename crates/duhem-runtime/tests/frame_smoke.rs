//! Real browser coverage for #568; two ports make the embed cross-origin.
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{Router, response::Html, routing::get};
use duhem_actions::{
    Action, ActionCtx, AssertElement, Click, Locator, Navigate, RunBrowser, to_selector,
};
use duhem_evidence::{SqliteStore, Trace};
use duhem_judge::VerdictState;
use duhem_runtime::{CapturePolicy, Engine};
use duhem_schema::VerificationDefinition;
use serde_yml::Value;
use tokio::{net::TcpListener, task::JoinHandle};

const EXAMPLE: &str = include_str!("../../../verifications/iframe-example/duhem.yml");
const REPORT: &str = r#"<!doctype html><html><body>
<main><h1>Weekly report</h1>
<label>Title<input></label>
<label>Format<select><option value="brief">Brief</option><option value="detailed">Detailed</option></select></label>
<button onclick="document.querySelector('[role=status]').textContent='Saved'">Save</button>
<div role="status"></div>
<iframe data-testid="nested" src="/inner"></iframe>
</main></body></html>"#;

struct Fixture {
    url: String,
    servers: Vec<JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
    }
}
async fn serve(app: Router) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, server)
}
async fn fixture() -> Fixture {
    let child = Router::new()
        .route("/", get(|| async { Html(REPORT) }))
        .route("/inner", get(|| async { Html("<h1>Nested report</h1>") }))
        .route(
            "/late",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(250)).await;
                Html(REPORT)
            }),
        );
    let (child_url, child_server) = serve(child).await;
    let parent = format!(
        r#"<!doctype html><html><body><h1>Parent only</h1>
<div data-testid="not-frame"></div>
<iframe data-testid="report-shell-iframe" src="{child_url}/" style="margin-left:120px;border:0;width:500px;height:500px"></iframe>
<iframe class="duplicate" src="{child_url}/"></iframe><iframe class="duplicate" src="{child_url}/"></iframe>
<iframe data-testid="local" src="/local"></iframe>
<button onclick="setTimeout(() => {{const f=document.createElement('iframe');f.dataset.testid='delayed';f.src='{child_url}/late';document.body.appendChild(f)}},100)">Load delayed frame</button>
</body></html>"#
    );
    let app = Router::new()
        .route(
            "/",
            get(move || {
                let html = parent.clone();
                async move { Html(html) }
            }),
        )
        .route("/local", get(|| async { Html("<h1>Local report</h1>") }));
    let (url, parent_server) = serve(app).await;
    Fixture {
        url,
        servers: vec![child_server, parent_server],
    }
}
fn yaml(raw: &str) -> Value {
    serde_yml::from_str(raw).unwrap()
}
async fn navigate(ctx: &ActionCtx<'_>, url: &str) {
    Navigate
        .invoke(
            ctx,
            &serde_yml::to_value(serde_json::json!({"url":url})).unwrap(),
        )
        .await
        .unwrap();
}
fn assertion(locator: &str, expected: &str) -> Value {
    yaml(&format!(
        "locator: {locator}\nexpected: {expected}\ntimeout: 400ms"
    ))
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn worked_vd_executes_cross_origin_named_locators_all_actions_and_evidence() {
    let fx = fixture().await;
    let def = VerificationDefinition::from_yaml_str(EXAMPLE).unwrap();
    let browser = RunBrowser::launch(false).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(tmp.path().join("runs.db")).await.unwrap());
    let mut engine = Engine::new()
        .with_browser(browser)
        .with_store(store.clone())
        .with_capture(CapturePolicy::Always);
    let outcome = engine
        .run_with_metadata(
            &def,
            BTreeMap::from([("fixture_url".into(), serde_json::json!(fx.url))]),
        )
        .await
        .unwrap();
    assert_eq!(
        outcome.verdict.state,
        VerdictState::Pass,
        "{:?}",
        outcome.verdict
    );
    let trace = Trace::from_store(store.as_ref(), &outcome.run_id)
        .await
        .unwrap();
    let recorded = serde_json::to_value(trace.events()).unwrap().to_string();
    assert!(recorded.contains("report-shell-iframe") && recorded.contains("Weekly report"));
    assert!(recorded.contains("capture/target-rect"));
    assert_eq!(
        duhem_evidence::replay(&trace).unwrap().run.state,
        VerdictState::Pass
    );
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn frame_contents_are_distinct_from_page_and_negative_assertions_discriminate() {
    let fx = fixture().await;
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    navigate(&ctx, &fx.url).await;
    let inner = "{frame: {testid: report-shell-iframe}, role: heading, name: Weekly report}";
    for (locator, expected, satisfied) in [
        (inner, "visible", true),
        (inner, "not_exists", false),
        ("{role: heading, name: Weekly report}", "visible", false),
        ("{role: heading, name: Weekly report}", "not_exists", true),
        (
            "{frame: {testid: report-shell-iframe}, role: heading, name: Missing}",
            "visible",
            false,
        ),
        (
            "{frame: {testid: local}, role: heading, name: Local report}",
            "visible",
            true,
        ),
        (
            "{frame: {frame: {testid: report-shell-iframe}, testid: nested}, role: heading, name: Nested report}",
            "visible",
            true,
        ),
        (
            "{frame: {testid: report-shell-iframe}, scope: {role: main}, text: Weekly report}",
            "visible",
            true,
        ),
    ] {
        let result = AssertElement
            .invoke(&ctx, &assertion(locator, expected))
            .await
            .unwrap();
        assert_eq!(
            result.outputs["satisfied"],
            serde_json::json!(satisfied),
            "{locator} / {expected}"
        );
    }
    let loc: Locator = serde_yml::from_str(inner).unwrap();
    let rect = check
        .page
        .bounding_box(&to_selector(&loc), 500.0)
        .await
        .unwrap()
        .unwrap();
    assert!(rect.x >= 120.0, "frame offset missing: {rect:?}");
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn unavailable_ambiguous_and_non_frame_targets_never_prove_inner_absence() {
    let fx = fixture().await;
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    navigate(&ctx, &fx.url).await;
    for frame in [
        "{testid: absent}",
        "{css: iframe.duplicate}",
        "{testid: not-frame}",
    ] {
        let locator = format!("{{frame: {frame}, role: heading, name: Parent only}}");
        let error = AssertElement
            .invoke(&ctx, &assertion(&locator, "not_exists"))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Frame") && error.contains("Parent only"),
            "{error}"
        );
    }
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn waits_for_late_frame_and_document_and_keeps_one_deadline() {
    let fx = fixture().await;
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    navigate(&ctx, &fx.url).await;
    Click
        .invoke(&ctx, &yaml("role: button\nname: Load delayed frame"))
        .await
        .unwrap();
    let started = Instant::now();
    // The delayed frame's initial about:blank document must not prove
    // absence before its HTTP document arrives with the heading present.
    let absent = AssertElement.invoke(&ctx, &yaml("locator: {frame: {testid: delayed}, role: heading, name: Weekly report}\nexpected: not_exists\ntimeout: 2s")).await.unwrap();
    assert_eq!(absent.outputs["satisfied"], serde_json::json!(false));
    let result = AssertElement.invoke(&ctx, &yaml("locator: {frame: {testid: delayed}, role: heading, name: Weekly report}\nexpected: visible\ntimeout: 2s")).await.unwrap();
    assert_eq!(result.outputs["satisfied"], serde_json::json!(true));
    assert!(started.elapsed() < Duration::from_secs(5));

    let started = Instant::now();
    let result = Click
        .invoke(
            &ctx,
            &yaml("frame: {testid: delayed}\nrole: button\nname: Missing\ntimeout: 250ms"),
        )
        .await
        .unwrap();
    assert!(matches!(result.outcome, duhem_actions::Outcome::Timeout));
    assert!(started.elapsed() < Duration::from_secs(1));
}
