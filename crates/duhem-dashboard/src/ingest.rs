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
        let supplied = headers
            .get("authorization")
            .map(|value| value.as_bytes())
            .unwrap_or_default();
        if !constant_time_eq(supplied, expected.as_bytes()) {
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
    let raw: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(raw) => raw,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let bundle: RunBundle = match serde_json::from_value(raw.clone()) {
        Ok(bundle) => bundle,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let known = match serde_json::to_value(&bundle) {
        Ok(known) => known,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if !contains_only_known_keys(&raw, &known) {
        let client_version = header("x-duhem-version").unwrap_or("unknown");
        let message = format!(
            "this run comes from a newer Duhem (client {client_version}); upgrade this server ({}) to at least the client's version",
            env!("CARGO_PKG_VERSION")
        );
        return (StatusCode::BAD_REQUEST, message).into_response();
    }
    match state.store.ingest_bundle(&bundle, hash).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(duhem_evidence::store::IngestError::Invalid(message)) => {
            (StatusCode::BAD_REQUEST, message).into_response()
        }
        Err(duhem_evidence::store::IngestError::Conflict(message)) => {
            (StatusCode::CONFLICT, message).into_response()
        }
        Err(duhem_evidence::store::IngestError::Store(_)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "could not store bundle").into_response()
        }
    }
}

/// Check that every raw object key exists in the corresponding known value.
fn contains_only_known_keys(raw: &serde_json::Value, known: &serde_json::Value) -> bool {
    match (raw, known) {
        (serde_json::Value::Object(raw), serde_json::Value::Object(known)) => {
            raw.iter().all(|(key, value)| {
                known
                    .get(key)
                    .is_some_and(|known| contains_only_known_keys(value, known))
            })
        }
        (serde_json::Value::Array(raw), serde_json::Value::Array(known)) => {
            raw.len() == known.len()
                && raw
                    .iter()
                    .zip(known)
                    .all(|(raw, known)| contains_only_known_keys(raw, known))
        }
        _ => true,
    }
}

/// Compare secrets without an early exit on the first differing byte.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    #[test]
    fn token_comparison() {
        assert!(super::constant_time_eq(b"Bearer t", b"Bearer t"));
        assert!(!super::constant_time_eq(b"Bearer t", b"Bearer u"));
        assert!(!super::constant_time_eq(b"Bearer", b"Bearer t"));
    }
}
