//! Score providers run over HTTP, through the normal CLI and evidence sink.
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router, routing::post};
use duhem_evidence::{EventPayload, SqliteStore, Store};
use serde_json::{Value, json};

const KEY: &str = "fixture-score-key-abcd1234";
const VD: &str = r#"verification: score isolation
inputs:
  key: {type: string, secret: true}
criteria:
  - id: AC-1
    description: mechanical command completes
    checks:
      - id: AC-1.1
        steps:
          - id: before
            uses: cli/invoke
            with: {command: [printf, candidate]}
          - id: quality
            uses: llm/score
            with:
              provider: PROVIDER
              base_url: BASE_URL
              model: MODEL
              api_key: $inputs.key
              rubric: Explain the quality of the response.
              observations: [$steps.before.outputs.stdout, {file: note.txt}]
              range: {min: 0, max: 10}
              timeout: TIMEOUT
          - id: after
            uses: cli/invoke
            with: {command: [printf, observed]}
        assertions:
          - "$steps.after.outputs.stdout == 'observed'"
"#;

#[derive(Default)]
struct Stub {
    requests: Mutex<Vec<(String, HeaderMap, Value)>>,
}

async fn reply(
    State(stub): State<Arc<Stub>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    stub.requests
        .lock()
        .unwrap()
        .push((uri.path().to_string(), headers, body.clone()));
    let model = body["model"].as_str().unwrap();
    if model == "slow" {
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if model == "http-error" {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": "fixture failure"})),
        );
    }
    let text = match model {
        "high" => json!({"score": 10, "reason": KEY}).to_string(),
        "low" => json!({"score": 0, "reason": "unclear"}).to_string(),
        "out-of-range" => json!({"score": 11, "reason": "invalid"}).to_string(),
        _ => "not JSON".into(),
    };
    let mut body = if uri.path().ends_with("/messages") {
        json!({"model": "reported-anthropic", "content": [{"type": "text", "text": text}]})
    } else {
        json!({"model": "reported-openai", "choices": [{"message": {"content": text}}]})
    };
    if model == "high" {
        body["fixture_padding"] = json!(format!("{}{KEY}", "x".repeat(5000)));
    }
    (StatusCode::OK, Json(body))
}

async fn run(
    root: &Path,
    name: &str,
    source: &str,
    options: &[&str],
) -> (Output, Vec<duhem_evidence::Event>) {
    let vd = root.join(format!("{name}.yml"));
    let db = root.join(format!("{name}.db"));
    std::fs::write(&vd, source).unwrap();
    let output = tokio::process::Command::from(Command::new(env!("CARGO_BIN_EXE_duhem")))
        .args(["run", "--no-live", "--reporter", "json", "--db"])
        .arg(&db)
        .arg(&vd)
        .args(options)
        .env_remove("DUHEM_SHARD")
        .env_remove("DUHEM_RUN_ID")
        .current_dir(root)
        .output()
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!("{error}: {}", String::from_utf8_lossy(&output.stderr));
    });
    let store = SqliteStore::open(db).await.unwrap();
    let events = store
        .run_events(value["run_id"].as_str().unwrap())
        .await
        .unwrap();
    (output, events)
}

#[tokio::test]
async fn providers_and_missing_scores_preserve_the_mechanical_exit_code() {
    let stub = Arc::new(Stub::default());
    let app = Router::new()
        .route("/v1/messages", post(reply))
        .route("/v1/chat/completions", post(reply))
        .with_state(stub.clone());
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", socket.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("note.txt"), "workspace observation").unwrap();
    let supplied = format!("key={KEY}");
    for (name, provider, model, timeout, options, outcome, evidence) in [
        (
            "high",
            "anthropic",
            "high",
            "1s",
            vec!["--inputs", supplied.as_str()],
            "ok",
            "reported-anthropic",
        ),
        (
            "low",
            "openai",
            "low",
            "1s",
            vec!["--inputs", supplied.as_str()],
            "ok",
            "reported-openai",
        ),
        (
            "range",
            "openai",
            "out-of-range",
            "1s",
            vec!["--inputs", supplied.as_str()],
            "error",
            "outside the declared range",
        ),
        (
            "parse",
            "anthropic",
            "malformed",
            "1s",
            vec!["--inputs", supplied.as_str()],
            "error",
            "did not return JSON",
        ),
        (
            "http",
            "openai",
            "http-error",
            "1s",
            vec!["--inputs", supplied.as_str()],
            "error",
            "HTTP 502",
        ),
        (
            "timeout",
            "openai",
            "slow",
            "300ms",
            vec!["--inputs", supplied.as_str()],
            "timeout",
            "request timed out",
        ),
        (
            "disabled",
            "openai",
            "high",
            "1s",
            vec!["--no-scores", "--inputs", supplied.as_str()],
            "skipped",
            "disabled by --no-scores",
        ),
        (
            "missing",
            "openai",
            "high",
            "1s",
            vec![],
            "error",
            "unresolved reference",
        ),
    ] {
        let source = VD
            .replace("PROVIDER", provider)
            .replace("BASE_URL", &endpoint)
            .replace("MODEL", model)
            .replace("TIMEOUT", timeout);
        let (output, events) = run(tmp.path(), name, &source, &options).await;
        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(summary["verdict"], "pass", "{name}: {summary}");
        assert_eq!(
            summary["totals"],
            json!({"total": 1, "passed": 1, "failed": 0, "inconclusive": 0}),
            "{name}: {summary}"
        );
        assert_eq!(summary["failures"], json!([]), "{name}: {summary}");
        let score = &summary["scores"]["AC-1"]["AC-1.1"]["quality"];
        match name {
            "high" => assert_eq!(score, &json!({"value": 10.0})),
            "low" => assert_eq!(score, &json!({"value": 0.0})),
            _ => assert!(
                score["missing_cause"]
                    .as_str()
                    .is_some_and(|cause| cause.contains(evidence)),
                "{name}: {score}"
            ),
        }
        if name == "high" || name == "disabled" {
            let out = tmp.path().join(format!("{name}-bundle"));
            let export = Command::new(env!("CARGO_BIN_EXE_duhem"))
                .args(["export", summary["run_id"].as_str().unwrap(), "--db"])
                .arg(tmp.path().join(format!("{name}.db")))
                .arg("--out")
                .arg(&out)
                .output()
                .unwrap();
            assert!(
                export.status.success(),
                "{}",
                String::from_utf8_lossy(&export.stderr)
            );
            let bundle = duhem_evidence::RunBundle::from_dir(&out).unwrap();
            assert_eq!(
                serde_json::to_value(&bundle.run.scores).unwrap(),
                summary["scores"]
            );
            if name == "high" {
                assert!(
                    !String::from_utf8(bundle.wire_bytes().unwrap())
                        .unwrap()
                        .contains(KEY)
                );
                assert_eq!(
                    bundle.artifacts.len(),
                    1,
                    "large raw response should be in the bundle"
                );
                for artifact in std::fs::read_dir(out.join("artifacts")).unwrap() {
                    let text = std::fs::read_to_string(artifact.unwrap().path()).unwrap();
                    assert!(
                        !text.contains(KEY),
                        "provider echoed the key into a bundle artifact"
                    );
                }
            }
        }
        let score_outcome = events
            .iter()
            .find_map(|event| match &event.payload {
                EventPayload::StepFinished {
                    step_index: 1,
                    outcome,
                    ..
                } => Some(serde_json::to_string(outcome).unwrap()),
                _ => None,
            })
            .unwrap();
        assert!(score_outcome.contains(outcome), "{name}: {score_outcome}");
        let serialized = serde_json::to_string(&events).unwrap();
        assert!(serialized.contains(evidence), "{name}: missing {evidence}");
        assert!(
            !serialized.contains(KEY),
            "{name}: key persisted in evidence"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(&event.payload, EventPayload::AssertionEvaluated { .. }))
                .count(),
            1
        );
        if name == "high" || name == "low" {
            assert!(serialized.contains("prompt_hash"));
            assert!(serialized.contains("raw_response"));
        }
    }
    {
        let requests = stub.requests.lock().unwrap();
        assert_eq!(requests.len(), 6);
        for (uri, headers, body) in requests.iter() {
            let prompt: Value =
                serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
            assert_eq!(
                prompt["observations"],
                json!(["candidate", "workspace observation"])
            );
            assert_eq!(body["temperature"], 0);
            assert!(!body.to_string().contains(KEY));
            if uri.ends_with("/messages") {
                assert_eq!(headers["x-api-key"], KEY);
                assert_eq!(headers["anthropic-version"], "2023-06-01");
            } else {
                assert_eq!(
                    headers["authorization"].to_str().unwrap(),
                    format!("Bearer {KEY}")
                );
            }
        }
    }
    let source = VD
        .replace("PROVIDER", "openai")
        .replace("BASE_URL", &endpoint)
        .replace("MODEL", "high")
        .replace("TIMEOUT", "1s")
        .replace(
            "$steps.after.outputs.stdout == 'observed'",
            "$inputs.key == 'required'",
        );
    let (output, _) = run(tmp.path(), "shared", &source, &["--no-scores"]).await;
    assert_ne!(output.status.code(), Some(0));
    server.abort();
}
