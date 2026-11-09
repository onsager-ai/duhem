//! Real browser coverage for #580 S2: `ui/back` / `ui/forward` over real history.
use std::collections::BTreeMap;

use axum::{Router, response::Html, routing::get};
use duhem_actions::{Action, ActionCtx, Back, Click, Forward, Navigate, Outcome, RunBrowser};
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use serde_json::json;
use tokio::{net::TcpListener, task::JoinHandle};

const EXAMPLE: &str = include_str!("../../../verifications/history-example/duhem.yml");
const LIST: &str = r##"<!doctype html><html><body>
<h1>List</h1><label>Draft<input></label>
<a href="/detail">Detail</a> <a href="#section">Jump</a>
</body></html>"##;

struct Fixture {
    url: String,
    server: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn fixture() -> Fixture {
    let app = Router::new()
        .route("/", get(|| async { Html(LIST) }))
        .route("/detail", get(|| async { Html("<h1>Detail</h1>") }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fixture { url, server }
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn worked_vd_executes_against_real_history() {
    let fx = fixture().await;
    let def = VerificationDefinition::from_yaml_str(EXAMPLE).unwrap();
    let browser = RunBrowser::launch(false).await.unwrap();
    let mut engine = Engine::new().with_browser(browser);
    let outcome = engine
        .run_with_metadata(
            &def,
            BTreeMap::from([("fixture_url".into(), json!(fx.url))]),
        )
        .await
        .unwrap();
    assert_eq!(
        outcome.verdict.state,
        VerdictState::Pass,
        "{:?}",
        outcome.verdict
    );
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn history_outputs_report_urls_and_navigated_for_every_entry_kind() {
    let fx = fixture().await;
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    let none = serde_yml::Value::Null;
    let list = format!("{}/", fx.url);
    let detail = format!("{}/detail", fx.url);

    Navigate
        .invoke(&ctx, &serde_yml::to_value(json!({"url": list})).unwrap())
        .await
        .unwrap();

    // No entry in either direction: not an error, URL unchanged.
    for result in [
        Back.invoke(&ctx, &none).await.unwrap(),
        Forward.invoke(&ctx, &none).await.unwrap(),
    ] {
        assert_eq!(result.outcome, Outcome::Ok);
        assert_eq!(result.outputs["navigated"], json!(false));
        assert_eq!(result.outputs["from_url"], json!(list));
        assert_eq!(result.outputs["url"], json!(list));
    }

    // Cross-document entry.
    let link = serde_yml::to_value(json!({"role": "link", "name": "Detail"})).unwrap();
    Click.invoke(&ctx, &link).await.unwrap();
    let back = Back.invoke(&ctx, &none).await.unwrap();
    assert_eq!(back.outputs["navigated"], json!(true));
    assert_eq!(back.outputs["from_url"], json!(detail));
    assert_eq!(back.outputs["url"], json!(list));
    let forward = Forward.invoke(&ctx, &none).await.unwrap();
    assert_eq!(forward.outputs["navigated"], json!(true));
    assert_eq!(forward.outputs["from_url"], json!(list));
    assert_eq!(forward.outputs["url"], json!(detail));

    // Same-document (hash) entry: Playwright returns null here as well as
    // for "no entry", so `navigated` must come from the navigation event.
    Back.invoke(&ctx, &none).await.unwrap();
    let jump = serde_yml::to_value(json!({"role": "link", "name": "Jump"})).unwrap();
    Click.invoke(&ctx, &jump).await.unwrap();
    let hash_back = Back.invoke(&ctx, &none).await.unwrap();
    assert_eq!(hash_back.outputs["navigated"], json!(true));
    assert_eq!(hash_back.outputs["from_url"], json!(format!("{list}#section")));
    assert_eq!(hash_back.outputs["url"], json!(list));
}
