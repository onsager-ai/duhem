//! Registered secrets in request URLs are masked in stored evidence
//! without changing the count or order of windowed `api/observe`
//! records (#603 P3). Needs Chromium; ignored by default.
use std::{collections::BTreeMap, sync::Arc};

use axum::{Router, response::Html, routing::get};
use duhem_actions::RunBrowser;
use duhem_evidence::{EventPayload, SqliteStore, Trace};
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use tokio::net::TcpListener;

const SECRET: &str = "tok-window-s3cr3t";
const VD: &str = r#"
verification: masked window
inputs:
  page_url: { type: string }
  token: { type: string, secret: true }
criteria:
  - id: AC-1
    description: Request URLs are masked in order
    checks:
      - id: AC-1.1
        steps:
          - id: load
            uses: ui/navigate
            with: { url: $inputs.page_url }
          - id: reads
            uses: api/observe
            with:
              url_pattern: "re:/ping"
              window: { from_step: load, duration: 1s }
        assertions:
          - $steps.reads.outputs.count == 3
          - $steps.reads.outputs.requests[0].url != 'unmatched'
          - $steps.reads.outputs.requests[1].url != 'unmatched'
          - $steps.reads.outputs.requests[2].url != 'unmatched'
"#;

#[tokio::test]
#[ignore = "requires Chromium"]
async fn secret_in_request_url_is_masked_without_reordering() {
    let page = format!(
        "<!doctype html><script>\
         fetch('/ping?n=1');\
         fetch('/ping?token={SECRET}&n=2');\
         fetch('/ping?n=3')</script>"
    );
    let app = Router::new()
        .route(
            "/",
            get(move || {
                let page = page.clone();
                async move { Html(page) }
            }),
        )
        .route("/ping", get(|| async { "ok" }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let def = VerificationDefinition::from_yaml_str(VD).unwrap();
    let browser = RunBrowser::launch(false).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(tmp.path().join("runs.db")).await.unwrap());
    let outcome = Engine::new()
        .with_browser(browser)
        .with_store(store.clone())
        .run_with_metadata(
            &def,
            BTreeMap::from([
                ("page_url".into(), serde_json::json!(url)),
                ("token".into(), serde_json::json!(SECRET)),
            ]),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(
        outcome.verdict.state,
        VerdictState::Pass,
        "{:?}",
        outcome.verdict
    );

    let trace = Trace::from_store(store.as_ref(), &outcome.run_id)
        .await
        .unwrap();
    let encoded = serde_json::to_string(trace.events()).unwrap();
    assert!(!encoded.contains(SECRET), "secret leaked into evidence");

    // Operands of the three URL assertions, in assertion order.
    let mut urls: Vec<(u32, String)> = trace
        .events()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::AssertionEvaluated {
                operands: Some(values),
                assertion_index,
                ..
            } if *assertion_index > 0 => Some((
                *assertion_index,
                values.values().next()?.as_str()?.to_string(),
            )),
            _ => None,
        })
        .collect();
    urls.sort();
    let urls: Vec<_> = urls.into_iter().map(|(_, url)| url).collect();
    assert_eq!(urls.len(), 3, "{urls:?}");
    assert!(urls[0].ends_with("/ping?n=1"), "{urls:?}");
    assert!(
        urls[1].contains("[redacted:token]") && urls[1].ends_with("&n=2"),
        "{urls:?}"
    );
    assert!(urls[2].ends_with("/ping?n=3"), "{urls:?}");
}
