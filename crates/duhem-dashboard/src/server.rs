//! Axum router for the dashboard: the #53/#85 JSON API, the #84 live
//! SSE endpoint, and a fallthrough that serves the embedded SPA
//! bundle (#86).
//!
//! Every handler re-queries the evidence store on each request — the
//! MVP's hot-reload posture. The server holds a read-only store
//! handle, owns no mutable state, and never invokes the runtime or
//! judge.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;
use serde_json::json;

use crate::live::live_stream;
use crate::reader::{EvidenceReader, ReaderError};

/// The Vite SPA bundle (#86), embedded at compile time. `build.rs`
/// guarantees the folder exists (placeholder index when the SPA
/// hasn't been built).
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

pub fn router(reader: EvidenceReader) -> Router {
    Router::new()
        .route("/api/runs", get(list_runs))
        .route("/api/runs.json", get(list_runs))
        .route("/api/runs/{run_id}", get(run_detail))
        .route("/api/runs/{run_id}/checks/{pair}", get(check_detail))
        .route("/api/runs/{run_id}/diff", get(run_diff))
        .route("/api/runs/{run_id}/diff.json", get(run_diff))
        .route("/api/runs/{run_id}/failure", get(failure_envelope))
        .route("/api/runs/{run_id}/failure.json", get(failure_envelope))
        .route("/api/runs/{run_id}/failure/{pair}", get(failing_check))
        .route("/api/runs/{run_id}/trace.jsonl", get(raw_trace))
        .route("/api/runs/{run_id}/artifact/{artifact_id}", get(artifact))
        .route("/api/runs/{run_id}/live", get(live))
        .route(
            "/api/verifications/{name}/history",
            get(verification_history),
        )
        // Static-export twin (the SPA always fetches the `.json`
        // spelling — same rationale as `strip_json_suffix`).
        .route(
            "/api/verifications/{name}/history.json",
            get(verification_history),
        )
        .fallback(get(static_asset))
        .with_state(reader)
}

fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, axum::Json(json!({ "error": msg.into() }))).into_response()
}

fn not_found(what: &str) -> Response {
    error_response(StatusCode::NOT_FOUND, format!("{what} not found"))
}

impl IntoResponse for ReaderError {
    fn into_response(self) -> Response {
        let status = match self {
            ReaderError::BadArtifactId(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        error_response(status, self.to_string())
    }
}

/// The SPA fetches `.json`-suffixed paths so one fetch layer works
/// against both the live server and a static export (where `api/runs`
/// must be a directory *and* a document — impossible on a plain file
/// host). The server accepts both spellings; the suffix-less paths
/// are the #53 contract, the `.json` twins are the static-export
/// mirror.
fn strip_json_suffix(s: &str) -> &str {
    s.strip_suffix(".json").unwrap_or(s)
}

async fn list_runs(State(reader): State<EvidenceReader>) -> Response {
    match reader.list().await {
        Ok(list) => axum::Json(list).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn run_detail(State(reader): State<EvidenceReader>, Path(run_id): Path<String>) -> Response {
    match reader.run_detail(strip_json_suffix(&run_id)).await {
        Ok(Some(detail)) => axum::Json(detail).into_response(),
        Ok(None) => not_found("run"),
        Err(e) => e.into_response(),
    }
}

async fn check_detail(
    State(reader): State<EvidenceReader>,
    Path((run_id, pair)): Path<(String, String)>,
) -> Response {
    let pair = strip_json_suffix(&pair);
    let Some((criterion_id, check_id)) = pair.split_once("::") else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "expected <criterion-id>::<check-id>",
        );
    };
    match reader.check_detail(&run_id, criterion_id, check_id).await {
        Ok(Some(detail)) => axum::Json(detail).into_response(),
        Ok(None) => not_found("check"),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/runs/:run_id/diff` (#211): the run vs its last-pass
/// baseline. `?baseline=<run-id>` pins a specific run instead of
/// auto-resolving.
#[derive(serde::Deserialize)]
struct DiffQuery {
    baseline: Option<String>,
}

async fn run_diff(
    State(reader): State<EvidenceReader>,
    Path(run_id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<DiffQuery>,
) -> Response {
    match reader
        .run_diff(strip_json_suffix(&run_id), q.baseline.as_deref())
        .await
    {
        Ok(Some(diff)) => axum::Json(diff).into_response(),
        Ok(None) => not_found("run"),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/runs/:run_id/failure` (#216): the machine-readable
/// failure envelope for an agent reacting to a `fail`.
async fn failure_envelope(
    State(reader): State<EvidenceReader>,
    Path(run_id): Path<String>,
) -> Response {
    match reader.failure_envelope(strip_json_suffix(&run_id)).await {
        Ok(Some(env)) => axum::Json(env).into_response(),
        Ok(None) => not_found("run"),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/runs/:run_id/failure/:crit::check` — the envelope scoped
/// to one check.
async fn failing_check(
    State(reader): State<EvidenceReader>,
    Path((run_id, pair)): Path<(String, String)>,
) -> Response {
    let run_id = strip_json_suffix(&run_id);
    let pair = strip_json_suffix(&pair);
    let Some((criterion_id, check_id)) = pair.split_once("::") else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "expected <criterion-id>::<check-id>",
        );
    };
    match reader.failing_check(run_id, criterion_id, check_id).await {
        Ok(Some(fc)) => axum::Json(fc).into_response(),
        Ok(None) => not_found("check"),
        Err(e) => e.into_response(),
    }
}

async fn raw_trace(State(reader): State<EvidenceReader>, Path(run_id): Path<String>) -> Response {
    match reader.raw_events_jsonl(&run_id).await {
        Ok(Some(jsonl)) => {
            ([(header::CONTENT_TYPE, "application/x-ndjson")], jsonl).into_response()
        }
        Ok(None) => not_found("run"),
        Err(e) => e.into_response(),
    }
}

async fn artifact(
    State(reader): State<EvidenceReader>,
    Path((run_id, artifact_id)): Path<(String, String)>,
) -> Response {
    match reader.artifact(&run_id, &artifact_id).await {
        Ok(Some((bytes, mime))) => ([(header::CONTENT_TYPE, mime)], bytes).into_response(),
        Ok(None) => not_found("artifact"),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/verifications/:name/history` (#193 ② VD-over-time).
async fn verification_history(
    State(reader): State<EvidenceReader>,
    Path(name): Path<String>,
) -> Response {
    match reader.verification_history(&name).await {
        Ok(Some(history)) => axum::Json(history).into_response(),
        Ok(None) => not_found("verification"),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/runs/:run_id/live` (#84): replay-then-follow SSE over
/// the run's event stream. Available for finished runs too — the
/// stream then replays to `run_finished` and closes immediately,
/// which keeps the client logic mode-free.
async fn live(State(reader): State<EvidenceReader>, Path(run_id): Path<String>) -> Response {
    match reader.store().get_run(&run_id).await {
        Ok(Some(_)) => Sse::new(live_stream(reader.store().clone(), run_id))
            .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
            .into_response(),
        Ok(None) => not_found("run"),
        Err(e) => ReaderError::from(e).into_response(),
    }
}

/// Serve the embedded SPA. Unknown non-`/api` paths fall back to
/// `index.html` so client-side routes deep-link cleanly.
async fn static_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return not_found("route");
    }
    let candidate = if path.is_empty() { "index.html" } else { path };
    let (file, name) = match Assets::get(candidate) {
        Some(f) => (f, candidate),
        None => match Assets::get("index.html") {
            Some(f) => (f, "index.html"),
            None => return not_found("asset"),
        },
    };
    (
        [(header::CONTENT_TYPE, mime_for(name))],
        file.data.into_owned(),
    )
        .into_response()
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("map") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Every embedded SPA asset, for the static exporter (#87). Returns
/// `(relative path, bytes)` pairs.
pub fn spa_assets() -> Vec<(String, Vec<u8>)> {
    Assets::iter()
        .filter_map(|name| Assets::get(&name).map(|f| (name.to_string(), f.data.into_owned())))
        .collect()
}
