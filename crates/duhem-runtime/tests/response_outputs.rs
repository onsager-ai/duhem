//! Executes `verifications/response-outputs-example` against a real local
//! HTTP server (#603 P1): declared `api/call` outputs and `body_bytes`.
use duhem_judge::VerdictState;
use duhem_runtime::Engine;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::routing::get;
use tokio::net::TcpListener;

const EXAMPLE: &str = include_str!("../../../verifications/response-outputs-example/duhem.yml");

async fn serve() -> String {
    let app = Router::new()
        .route(
            "/resource",
            get(|| async {
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                        (header::LOCATION, "received"),
                    ],
                    "é",
                )
            }),
        )
        .route("/empty", get(|| async { StatusCode::OK }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn run(yaml: &str, base: &str) -> VerdictState {
    let def = duhem_schema::VerificationDefinition::from_yaml_str(yaml).unwrap();
    duhem_schema::validate_with_action_catalog(&def, &|uses| {
        duhem_actions::contract_for(uses).map(|c| c.outputs.iter().map(|o| o.to_string()).collect())
    })
    .unwrap();
    let inputs = [("resource_url", "resource"), ("empty_url", "empty")]
        .into_iter()
        .map(|(name, path)| {
            (
                name.to_string(),
                serde_json::Value::String(format!("{base}/{path}")),
            )
        })
        .collect();
    // A private store per run: parallel tests must not share the
    // working-copy evidence DB (SQLite "database is locked").
    let tmp = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(
        duhem_evidence::SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .unwrap(),
    );
    Engine::new()
        .with_store(store)
        .run(&def, inputs)
        .await
        .unwrap()
        .state
}

#[tokio::test]
async fn response_outputs_example_passes_against_real_server() {
    let base = serve().await;
    assert_eq!(run(EXAMPLE, &base).await, VerdictState::Pass);
}

#[tokio::test]
async fn wrong_byte_count_fails() {
    let base = serve().await;
    let yaml = EXAMPLE.replace("body_bytes == 2", "body_bytes == 1");
    assert_eq!(run(&yaml, &base).await, VerdictState::Fail);
}
