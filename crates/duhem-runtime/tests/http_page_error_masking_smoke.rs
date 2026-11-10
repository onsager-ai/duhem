//! Registered secrets in HTTP header values and page errors stay masked
//! in action outputs, stored evidence and exported bundles (#603).
use std::{collections::BTreeMap, sync::Arc};

use axum::{Router, http::HeaderMap, response::Html, routing::get};
use duhem_actions::RunBrowser;
use duhem_evidence::{EventPayload, RunBundle, SqliteStore, Trace};
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use tokio::net::TcpListener;

const SECRET: &str = "masking-header-page-error-603";
const VD: &str = r#"
verification: HTTP and page error masking
inputs:
  page_url: { type: string }
  call_url: { type: string }
  token: { type: string, secret: true }
criteria:
  - id: AC-1
    description: Header values and page errors are masked without losing observations
    checks:
      - id: AC-1.1
        steps:
          - id: load
            uses: ui/navigate
            with: { url: $inputs.page_url }
          - id: observed
            uses: api/observe
            with: { url_pattern: "re:/observed$", timeout: 3s }
          - id: errors
            uses: ui/observe-console
            with: { window: { from_step: load, duration: 1s } }
          - id: called
            uses: api/call
            with:
              method: GET
              url: $inputs.call_url
              headers: { x-client-token: $inputs.token }
        assertions:
          - $steps.observed.outputs.request_headers['x-client-token'] != 'unmatched'
          - $steps.observed.outputs.headers['x-response-token'] != 'unmatched'
          - $steps.errors.outputs.page_error_count == 1
          - $steps.errors.outputs.page_errors[0].message != 'unmatched'
          - $steps.called.outputs.headers['x-response-token'] != 'unmatched'
"#;

#[tokio::test]
#[ignore = "requires Chromium"]
async fn http_headers_and_page_error_are_masked_in_outputs_evidence_and_export() {
    let page = format!(
        "<!doctype html><script>fetch('/observed', {{headers: {{'x-client-token': '{SECRET}'}}}}); setTimeout(() => {{ throw new Error('{SECRET}') }}, 50)</script>"
    );
    let app = Router::new()
        .route(
            "/",
            get(move || {
                let page = page.clone();
                async move { Html(page) }
            }),
        )
        .route(
            "/observed",
            get(|headers: HeaderMap| async move {
                let request_value = headers
                    .get("x-client-token")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("missing")
                    .to_owned();
                ([ ("x-response-token", request_value) ], "ok")
            }),
        )
        .route(
            "/call",
            get(|| async { ([ ("x-response-token", SECRET) ], "ok") }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let browser = RunBrowser::launch(false).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(tmp.path().join("runs.db")).await.unwrap());
    let result = Engine::new()
        .with_browser(browser)
        .with_store(store.clone())
        .run_with_metadata(
            &VerificationDefinition::from_yaml_str(VD).unwrap(),
            BTreeMap::from([
                ("page_url".into(), serde_json::json!(format!("{base}/"))),
                ("call_url".into(), serde_json::json!(format!("{base}/call"))),
                ("token".into(), serde_json::json!(SECRET)),
            ]),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result.verdict.state, VerdictState::Pass, "{:?}", result.verdict);

    let trace = Trace::from_store(store.as_ref(), &result.run_id).await.unwrap();
    let trace_json = serde_json::to_string(trace.events()).unwrap();
    assert!(!trace_json.contains(SECRET), "secret leaked into stored trace");
    let observations: Vec<_> = trace
        .events()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::StepObservation { output_name, value: duhem_evidence::ObservationValue::Inline { value }, .. } => {
                Some((output_name.as_str(), value))
            }
            _ => None,
        })
        .collect();
    let request_headers = observations.iter().find(|(name, _)| *name == "request_headers").unwrap().1;
    let response_headers = observations.iter().filter(|(name, _)| *name == "headers").count();
    let errors = observations.iter().find(|(name, _)| *name == "page_errors").unwrap().1;
    assert_eq!(request_headers["x-client-token"], "[redacted:token]");
    assert!(response_headers >= 2);
    assert_eq!(errors.as_array().unwrap().len(), 1);
    assert!(errors[0]["message"].as_str().unwrap().contains("[redacted:token]"));
    assert!(errors[0]["stack"].as_str().unwrap().contains("[redacted:token]"));

    let bundle = RunBundle::from_store(store.as_ref(), &result.run_id).await.unwrap();
    let exported = String::from_utf8(bundle.wire_bytes().unwrap()).unwrap();
    assert!(!exported.contains(SECRET), "secret leaked into exported bundle");
}
