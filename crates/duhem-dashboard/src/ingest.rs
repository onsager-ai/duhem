//! Explicitly enabled, bounded run-bundle replication endpoint.
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::post};
use duhem_evidence::{RunBundle, SqliteStore};

/// Limit includes JSON and base64 artifacts. Larger bundles need a future transport.
pub const MAX_BUNDLE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
struct IngestState {
    store: Arc<SqliteStore>,
    token: Option<String>,
}

/// Merge with the read-only reader router only when the operator opts in.
pub fn ingest_router(store: Arc<SqliteStore>, token: Option<String>) -> Router {
    Router::new()
        .route("/api/ingest", post(ingest))
        .layer(DefaultBodyLimit::max(MAX_BUNDLE_BYTES))
        .with_state(IngestState { store, token })
}

async fn ingest(State(state): State<IngestState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(token) = &state.token {
        let expected = format!("Bearer {token}");
        if headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            != Some(expected.as_str())
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    if header("x-duhem-bundle-version") != Some("1")
        || header("content-type").is_none_or(|value| !value.starts_with("application/json"))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(hash) = header("x-duhem-content-hash") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let bundle: RunBundle = match serde_json::from_slice(&body) {
        Ok(bundle) => bundle,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.store.ingest_bundle(&bundle, hash).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(duhem_evidence::store::IngestError::Invalid(message)) => {
            (StatusCode::BAD_REQUEST, message).into_response()
        }
        Err(duhem_evidence::store::IngestError::Store(_)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "could not store bundle").into_response()
        }
    }
}
