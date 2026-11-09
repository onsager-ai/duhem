//! Real browser coverage for #580 S4: `ui/extract` `field: rect` and `style:`.
use std::{collections::BTreeMap, sync::Arc};

use axum::{Router, response::Html, routing::get};
use duhem_actions::{Action, ActionCtx, Extract, Navigate, RunBrowser};
use duhem_evidence::SqliteStore;
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use serde_json::json;
use tokio::{net::TcpListener, task::JoinHandle};

const EXAMPLE: &str = include_str!("../../../verifications/geometry-example/duhem.yml");
const PAGE: &str = r#"<!doctype html><html><body style="margin:0">
<div data-testid="wide" style="display:block;width:200px;height:50px;margin-left:10px;background:#111"></div>
<div data-testid="narrow" style="display:block;width:80px;height:30px;background:#f00"></div>
<iframe data-testid="report-frame" src="/inner" style="margin-left:120px;margin-top:100px;border:0;width:300px;height:200px"></iframe>
</body></html>"#;
const INNER: &str = r#"<!doctype html><html><body style="margin:0">
<div data-testid="box" style="position:absolute;left:30px;top:20px;width:50px;height:40px;background:rgb(0,128,0)"></div>
</body></html>"#;

async fn serve() -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/inner", get(|| async { Html(INNER) }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, server)
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn worked_vd_executes_rect_and_style_with_controls() {
    let (url, server) = serve().await;
    let def = VerificationDefinition::from_yaml_str(EXAMPLE).unwrap();
    let browser = RunBrowser::launch(false).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(tmp.path().join("runs.db")).await.unwrap());
    let mut engine = Engine::new().with_browser(browser).with_store(store);
    let outcome = engine
        .run_with_metadata(&def, BTreeMap::from([("fixture_url".into(), json!(url))]))
        .await
        .unwrap();
    server.abort();
    assert_eq!(
        outcome.verdict.state,
        VerdictState::Pass,
        "{:?}",
        outcome.verdict
    );
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn frame_rect_is_frame_relative_and_page_rect_is_not() {
    let (url, server) = serve().await;
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
        step_boundaries: &std::collections::BTreeMap::new(),
        session: None,
    };
    Navigate
        .invoke(&ctx, &serde_yml::from_str(&format!("url: {url}")).unwrap())
        .await
        .unwrap();
    let frame = Extract
        .invoke(
            &ctx,
            &serde_yml::from_str(
                "locator: { frame: { testid: report-frame }, testid: box }\nfield: rect",
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let rect = frame.outputs.get("value").unwrap();
    assert_eq!(rect["x"], json!(30));
    assert_eq!(rect["y"], json!(20));
    let iframe = Extract
        .invoke(
            &ctx,
            &serde_yml::from_str("locator: { testid: report-frame }\nfield: rect").unwrap(),
        )
        .await
        .unwrap();
    let rect = iframe.outputs.get("value").unwrap();
    assert_eq!(rect["x"], json!(120));
    server.abort();
}
