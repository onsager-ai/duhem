//! End-to-end smoke for functional `outputs:` extraction (spec #273).
//!
//! Drives the full pipeline — parser → engine → `api/call` →
//! evaluator → judge — against an in-process `axum` server that
//! returns a nested JSON body. The single check binds three aliases
//! via `outputs:` — a rename (`http_code: status`), a deep extraction
//! (`project_id: body.data._id`), and an array-index extraction
//! (`first_item: body.items[0].id`) — and every assertion references
//! *only the aliases*, none of which is a native `api/call` output.
//! So a pass proves the map is doing real work: if extraction were
//! still inert the aliases would be `MissingObservation` and the
//! verdict could not be `Pass`. One assertion also reaches the raw
//! `status` to confirm aliases are additive, not a replacement.
//!
//! `#[ignore]`'d in CI for the same reason as `api_smoke.rs`: the
//! per-check browser launch needs `npx playwright install chromium`.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Json;
use axum::Router;
use axum::routing::post;
use duhem_actions::RunBrowser;
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const VD_YAML: &str = r#"
verification: outputs extraction smoke
inputs:
  create_url:
    type: string
criteria:
  - id: AC-1
    description: rename plus derived extraction resolve as aliases
    checks:
      - id: AC-1.1
        steps:
          - id: create
            uses: api/call
            with:
              method: POST
              url: $inputs.create_url
              headers:
                Content-Type: application/json
              body:
                name: demo
              within: 2s
            outputs:
              http_code: status
              project_id: body.data._id
              first_item: body.items[0].id
        assertions:
          - $steps.create.outputs.http_code == 200
          - $steps.create.outputs.project_id == "abc123"
          - $steps.create.outputs.first_item == "a"
          - $steps.create.outputs.status == 200
"#;

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

async fn start_fixture() -> Fixture {
    let app = Router::new().route(
        "/projects",
        post(|| async {
            Json(json!({
                "data": { "_id": "abc123" },
                "items": [ { "id": "a" }, { "id": "b" } ]
            }))
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
async fn outputs_map_rename_and_extraction_resolve_as_aliases() {
    let fx = start_fixture().await;
    let create_url = format!("http://{}/projects", fx.addr);

    let def = VerificationDefinition::from_yaml_str(VD_YAML).expect("parse VD");

    let browser = RunBrowser::launch(false)
        .await
        .expect("launch chromium (run `npx playwright install chromium`)");

    let tmp = tempfile::tempdir().expect("tempdir");
    let store = std::sync::Arc::new(
        duhem_evidence::SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .expect("open store"),
    );
    let mut engine = Engine::new()
        .with_browser(browser)
        .with_store(store.clone());

    let mut inputs = BTreeMap::new();
    inputs.insert(
        "create_url".to_string(),
        serde_json::Value::String(create_url),
    );

    let verdict = engine.run(&def, inputs).await.expect("engine.run");
    assert_eq!(
        verdict.state,
        VerdictState::Pass,
        "aliases must resolve end-to-end; verdict = {verdict:?}"
    );
}
