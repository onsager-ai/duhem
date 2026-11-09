//! Execute the console example and exercise registered-secret masking.
use std::{collections::BTreeMap, sync::Arc};

use axum::{Router, response::Html, routing::get};
use duhem_actions::RunBrowser;
use duhem_evidence::{SqliteStore, Trace};
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use tokio::net::TcpListener;

const EXAMPLE: &str = include_str!("../../../verifications/observe-console-example/duhem.yml");
const SECRET: &str = "console-secret-value-603";
const MASKING: &str = r#"
verification: Console masking
inputs:
  page_url: { type: string }
  token: { type: string, secret: true }
criteria:
  - id: AC-1
    description: Console text is masked
    checks:
      - id: AC-1.1
        steps:
          - id: load
            uses: ui/navigate
            with: { url: $inputs.page_url }
          - id: observed
            uses: ui/observe-console
            with: { window: { from_step: load, duration: 1s } }
        assertions:
          - $steps.observed.outputs.complete == true
          - $steps.observed.outputs.console_count == 3
          - $steps.observed.outputs.console[0].text == 'first'
          - $steps.observed.outputs.console[1].text != 'unmatched'
          - $steps.observed.outputs.console[2].text == 'last'
"#;

#[tokio::test]
#[ignore = "requires Chromium"]
async fn example_and_masked_console_records() {
    let page = format!(
        "<script>console.log('first'); console.log('{SECRET}'); console.log('last')</script>"
    );
    let app = Router::new()
        .route("/favicon.ico", get(|| async { "" }))
        .route(
            "/clean",
            get(|| async { Html("<!doctype html><title>clean</title>") }),
        )
        .route(
            "/error",
            get(|| async { Html("<script>console.error('control')</script>") }),
        )
        .route(
            "/secret",
            get(move || {
                let page = page.clone();
                async move { Html(page) }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let browser = RunBrowser::launch(false).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(tmp.path().join("runs.db")).await.unwrap());
    let mut engine = Engine::new()
        .with_browser(browser)
        .with_store(store.clone());
    let example = VerificationDefinition::from_yaml_str(EXAMPLE).unwrap();
    let result = engine
        .run_with_metadata(
            &example,
            BTreeMap::from([
                (
                    "clean_url".into(),
                    serde_json::json!(format!("{base}/clean")),
                ),
                (
                    "error_url".into(),
                    serde_json::json!(format!("{base}/error")),
                ),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(
        result.verdict.state,
        VerdictState::Pass,
        "{:?}",
        result.verdict
    );
    let masking = VerificationDefinition::from_yaml_str(MASKING).unwrap();
    let result = engine
        .run_with_metadata(
            &masking,
            BTreeMap::from([
                (
                    "page_url".into(),
                    serde_json::json!(format!("{base}/secret")),
                ),
                ("token".into(), serde_json::json!(SECRET)),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(
        result.verdict.state,
        VerdictState::Pass,
        "{:?}",
        result.verdict
    );
    let trace = Trace::from_store(store.as_ref(), &result.run_id)
        .await
        .unwrap();
    let encoded = serde_json::to_string(trace.events()).unwrap();
    assert!(!encoded.contains(SECRET));
    assert!(encoded.contains("[redacted:token]"));
    server.abort();
}
