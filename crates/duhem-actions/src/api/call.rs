//! `api/call` — active HTTP request against a real server.
//!
//! Drives one request per invocation via `reqwest` (rustls-backed,
//! sync-on-Tokio). The action ignores [`ActionCtx::page`] entirely:
//! per spec the check still opens a `CheckBrowser` so the runtime's
//! per-check lifecycle stays uniform across `ui/*` and `api/*`, but
//! `api/call` never touches the browser. Stripping the browser for
//! API-only Verification Definitions is an optimization deferred to
//! a later spec.
//!
//! Outputs surfaced (fixed schema):
//!
//! - `status`: response status code as an integer (u16 widened).
//! - `body`: parsed JSON value when `Content-Type` starts with
//!   `application/json`; `null` otherwise.
//! - `body_text`: raw response bytes as UTF-8 (lossy).
//! - `headers`: response headers as a JSON object (values rendered
//!   via UTF-8 lossy from the raw header bytes so non-ASCII /
//!   opaque values are still represented).
//!
//! The runtime evaluator (`duhem-runtime` issue #15) only records
//! *scalar* outputs into the expression context — JSON object and
//! array values, including `body` (when parsed as JSON) and
//! `headers`, land in the evidence trace but are not yet reachable
//! from `$steps.<id>.outputs.<name>` in an assertion. Plan for v1
//! assertions over the scalar outputs (`status`, `body_text`);
//! nested navigation into `body` requires an evaluator extension
//! that is its own spec.
//!
//! Template substitution in `Step.with` resolves whole-string
//! `$inputs.<name>` and `$runtime.<helper>()` references; it does
//! *not* perform string concatenation. Authors who need to compose
//! a URL from a base + path should pass the full URL as a single
//! input (`$inputs.echo_url`), not `$inputs.base_url + "/echo"`.
//!
//! Outcome mapping:
//!
//! - HTTP completes within `within:` → `Outcome::Ok`. The status code
//!   is data on the response, not a verdict — a `500` response is
//!   still `Outcome::Ok` from the action's standpoint, and the
//!   assertion is where `200 vs. 500` gets judged. Same shape as
//!   `ui/click` against a button that triggers a 500 page.
//! - `within:` exceeded → `Outcome::Timeout`.
//! - DNS / TCP / TLS / malformed method / malformed URL / non-string
//!   body keys → `ActionError::Http`, which the engine maps to
//!   `Outcome::Error`.
//!
//! `api/observe` (passive request sniffing) is documented in
//! `docs/duhem-spec.md` §10.5 and is a separate spec.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Method;
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN, Observation};
use crate::error::ActionError;
use crate::with::WithinSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct With {
    method: String,
    url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    /// Optional query parameters, appended to `url` as a urlencoded
    /// `?k=v&...` string. Scalars only (string/int/bool/float). The
    /// `BTreeMap` yields keys in sorted order so the rendered query
    /// string — and the evidence it lands in — is reproducible. If
    /// `url` already carries a `?query`, these are merged onto it with
    /// `&`. Saves authors from hand-building pagination/filter URLs
    /// with `$runtime.format`.
    #[serde(default)]
    query: BTreeMap<String, QueryScalar>,
    /// Request body. A YAML mapping/sequence/scalar (other than a
    /// String) is serialized as JSON; a YAML string is sent verbatim.
    /// `None` means no body.
    #[serde(default)]
    body: Option<serde_yml::Value>,
    #[serde(default)]
    within: Option<WithinSpec>,
}

/// A single query-parameter value. Restricted to scalars — a query
/// string has no representation for nested objects or sequences. The
/// `untagged` variant order matters: `bool` and the integer kinds are
/// tried before `String` so `page: 1` / `active: true` keep their
/// natural YAML type instead of being swallowed as strings, and the
/// final `String` arm catches everything else (including quoted
/// `"10"`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum QueryScalar {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl QueryScalar {
    /// Render the value as the raw (pre-urlencode) string that goes on
    /// the wire. Encoding is applied later by [`build_url`].
    fn to_param_string(&self) -> String {
        match self {
            QueryScalar::Bool(b) => b.to_string(),
            QueryScalar::Int(i) => i.to_string(),
            QueryScalar::Float(f) => f.to_string(),
            QueryScalar::Str(s) => s.clone(),
        }
    }
}

/// Append `query` to `url` as a urlencoded `?k=v&...` string.
///
/// - An empty `query` returns `url` unchanged (byte-identical to a
///   request authored without a `query:` block).
/// - Keys are emitted in `BTreeMap` (sorted) order for reproducible
///   evidence.
/// - Values are application/x-www-form-urlencoded (space → `+`,
///   reserved bytes percent-escaped) via `form_urlencoded`, matching
///   the convention `reqwest`'s own `.query()` uses.
/// - If `url` already contains a `?`, the new pairs are joined with
///   `&` (or appended directly when `url` ends in a bare `?`).
fn build_url(url: &str, query: &BTreeMap<String, QueryScalar>) -> String {
    if query.is_empty() {
        return url.to_string();
    }
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (k, v) in query {
        serializer.append_pair(k, &v.to_param_string());
    }
    let rendered = serializer.finish();
    let separator = match url.find('?') {
        None => "?",
        // `url` already ends with a bare `?` — no existing pair to
        // join against, so append the rendered string directly.
        Some(idx) if idx + 1 == url.len() => "",
        Some(_) => "&",
    };
    format!("{url}{separator}{rendered}")
}

pub struct Call;

#[async_trait]
impl Action for Call {
    fn uses(&self) -> &'static str {
        "api/call"
    }

    async fn invoke(
        &self,
        _ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "api/call",
                source: e,
            })?;
        execute(with).await
    }
}

/// Performs the HTTP call. Factored out from `Action::invoke` so the
/// network behavior can be unit-tested without constructing a
/// Playwright `Page`.
pub(crate) async fn execute(with: With) -> Result<ActionResult, ActionError> {
    let timeout: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);

    // Uppercase ASCII before parsing so authors who type `get` /
    // `post` get the conventional `GET` / `POST` instead of a
    // server-side surprise — `reqwest::Method::from_bytes` happily
    // accepts lowercase as a custom extension method, which most
    // servers don't recognize. Non-ASCII inputs fall through and
    // are rejected by `from_bytes`.
    let method_normalized = if with.method.is_ascii() {
        with.method.to_ascii_uppercase()
    } else {
        with.method.clone()
    };
    let method = Method::from_bytes(method_normalized.as_bytes()).map_err(|e| {
        ActionError::Http(format!(
            "api/call: invalid method `{}`: {e}",
            with.method.as_str()
        ))
    })?;

    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| ActionError::Http(format!("api/call: build client: {e}")))?;

    let url = build_url(&with.url, &with.query);
    let mut req = client.request(method, &url);
    for (k, v) in &with.headers {
        req = req.header(k, v);
    }
    if let Some(body) = with.body {
        req = match body {
            serde_yml::Value::String(s) => req.body(s),
            other => {
                let json = yml_to_json(&other)?;
                let bytes = serde_json::to_vec(&json)
                    .map_err(|e| ActionError::Http(format!("api/call: serialize body: {e}")))?;
                req.body(bytes)
            }
        };
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return Ok(ActionResult::timeout()),
        Err(e) => return Err(ActionError::Http(format!("api/call: {e}"))),
    };

    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    // Render header values via UTF-8 lossy from the raw bytes so a
    // header that includes a `0xFF` (legal in HTTP/1.1) still
    // appears in the `headers` output — silently dropping it would
    // erase contract-relevant data from the trace.
    let headers: BTreeMap<String, String> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ActionError::Http(format!("api/call: read body: {e}")))?;
    let body_text = String::from_utf8_lossy(&bytes).into_owned();
    let mut observations: Vec<Observation> = Vec::new();
    let body_json: serde_json::Value = if content_type.starts_with("application/json") {
        match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(v) => v,
            Err(e) => {
                // The server *claimed* JSON but sent something the
                // parser couldn't accept. Preserve the debug signal
                // as an observation rather than masking it as a
                // legitimate `null` response; assertions over
                // `body_text` still work.
                observations.push(Observation {
                    kind: "api.json_parse_failure".to_string(),
                    note: Some(format!(
                        "response body declared application/json but failed to parse: {e}"
                    )),
                });
                serde_json::Value::Null
            }
        }
    } else {
        serde_json::Value::Null
    };

    let mut result = ActionResult::ok()
        .with_output("status", serde_json::Value::from(status))
        .with_output("body", body_json)
        .with_output("body_text", serde_json::Value::String(body_text))
        .with_output(
            "headers",
            serde_json::Value::Object(
                headers
                    .into_iter()
                    .map(|(k, v)| (k, serde_json::Value::String(v)))
                    .collect(),
            ),
        );
    result.observations.append(&mut observations);
    Ok(result)
}

/// Convert a YAML value to a JSON value for outgoing request bodies.
/// Non-string mapping keys are rejected explicitly: JSON requires
/// string keys, and silently coercing or dropping them would produce
/// a body that differs from what the author wrote.
pub(crate) fn yml_to_json(v: &serde_yml::Value) -> Result<serde_json::Value, ActionError> {
    use serde_yml::Value as Y;
    Ok(match v {
        Y::Null => serde_json::Value::Null,
        Y::Bool(b) => serde_json::Value::Bool(*b),
        Y::Number(n) => serde_json::to_value(n).unwrap_or(serde_json::Value::Null),
        Y::String(s) => serde_json::Value::String(s.clone()),
        Y::Sequence(seq) => {
            serde_json::Value::Array(seq.iter().map(yml_to_json).collect::<Result<Vec<_>, _>>()?)
        }
        Y::Mapping(m) => {
            let mut obj = serde_json::Map::with_capacity(m.len());
            for (k, v) in m.iter() {
                let key = k.as_str().ok_or_else(|| {
                    ActionError::Http(format!(
                        "api/call: body has a non-string mapping key (got {}); JSON requires string keys",
                        yml_kind(k)
                    ))
                })?;
                obj.insert(key.to_string(), yml_to_json(v)?);
            }
            serde_json::Value::Object(obj)
        }
        Y::Tagged(t) => yml_to_json(&t.value)?,
    })
}

fn yml_kind(v: &serde_yml::Value) -> &'static str {
    use serde_yml::Value as Y;
    match v {
        Y::Null => "null",
        Y::Bool(_) => "bool",
        Y::Number(_) => "number",
        Y::String(_) => "string",
        Y::Sequence(_) => "sequence",
        Y::Mapping(_) => "mapping",
        Y::Tagged(_) => "tagged",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::net::SocketAddr;
    use std::time::Duration;

    use axum::Router;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{any, post};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    fn yaml(s: &str) -> serde_yml::Value {
        serde_yml::from_str(s).unwrap()
    }

    fn parse_with(s: &str) -> With {
        serde_yml::from_value(yaml(s)).expect("With deserialization")
    }

    // --- With deserialization ----------------------------------------

    #[test]
    fn parses_minimal_get() {
        let w = parse_with(r#"{ method: GET, url: "http://x/" }"#);
        assert_eq!(w.method, "GET");
        assert_eq!(w.url, "http://x/");
        assert!(w.headers.is_empty());
        assert!(w.query.is_empty());
        assert!(w.body.is_none());
        assert!(w.within.is_none());
    }

    #[test]
    fn parses_full_with() {
        let w = parse_with(
            r#"
method: POST
url: "http://x/y"
headers:
  Content-Type: application/json
  Authorization: "Bearer t"
body:
  hello: world
within: 3s
"#,
        );
        assert_eq!(w.method, "POST");
        assert_eq!(
            w.headers.get("Content-Type").map(String::as_str),
            Some("application/json")
        );
        let body = w.body.as_ref().expect("body");
        assert!(body.is_mapping());
        let d: Duration = w.within.unwrap().into();
        assert_eq!(d, Duration::from_secs(3));
    }

    #[test]
    fn rejects_unknown_field() {
        let r: Result<With, _> = serde_yml::from_str(r#"{ method: GET, url: "x", color: red }"#);
        assert!(r.is_err());
    }

    // --- query parameters --------------------------------------------

    #[test]
    fn query_parses_scalar_types() {
        let w = parse_with(
            r#"
method: GET
url: "http://x/"
query:
  page: 1
  size: 10
  active: true
  ratio: 1.5
  q: "hello world"
"#,
        );
        assert_eq!(w.query.len(), 5);
        assert!(matches!(w.query.get("page"), Some(QueryScalar::Int(1))));
        assert!(matches!(
            w.query.get("active"),
            Some(QueryScalar::Bool(true))
        ));
        assert!(matches!(w.query.get("ratio"), Some(QueryScalar::Float(_))));
        match w.query.get("q") {
            Some(QueryScalar::Str(s)) => assert_eq!(s, "hello world"),
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[test]
    fn build_url_absent_query_is_byte_identical() {
        let empty = BTreeMap::new();
        assert_eq!(build_url("http://x/list", &empty), "http://x/list");
        // Even a url that already carries a `?query` is untouched when
        // no `query:` block is supplied.
        assert_eq!(build_url("http://x/list?a=1", &empty), "http://x/list?a=1");
    }

    #[test]
    fn build_url_sorts_keys_and_coerces_scalars() {
        // Insertion order is deliberately unsorted; BTreeMap iteration
        // (and therefore the rendered query) must come out sorted.
        let mut q = BTreeMap::new();
        q.insert("size".to_string(), QueryScalar::Int(10));
        q.insert("page".to_string(), QueryScalar::Int(1));
        q.insert("active".to_string(), QueryScalar::Bool(true));
        assert_eq!(
            build_url("http://x/list", &q),
            "http://x/list?active=true&page=1&size=10"
        );
    }

    #[test]
    fn build_url_urlencodes_special_chars() {
        let mut q = BTreeMap::new();
        q.insert("q".to_string(), QueryScalar::Str("a b&c=d".to_string()));
        // Space → `+`, `&` and `=` percent-escaped (form-urlencoded).
        assert_eq!(build_url("http://x/s", &q), "http://x/s?q=a+b%26c%3Dd");
    }

    #[test]
    fn build_url_merges_with_existing_query() {
        let mut q = BTreeMap::new();
        q.insert("page".to_string(), QueryScalar::Int(2));
        // Existing `?token=t` is preserved; the new pair joins with `&`.
        assert_eq!(
            build_url("http://x/list?token=t", &q),
            "http://x/list?token=t&page=2"
        );
        // A url ending in a bare `?` gets the pairs appended directly.
        assert_eq!(build_url("http://x/list?", &q), "http://x/list?page=2");
    }

    // --- network behavior --------------------------------------------

    struct Fixture {
        addr: SocketAddr,
        _server: JoinHandle<()>,
    }

    async fn start(router: Router) -> Fixture {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Fixture {
            addr,
            _server: server,
        }
    }

    fn url(fx: &Fixture, path: &str) -> String {
        format!("http://{}{}", fx.addr, path)
    }

    #[tokio::test]
    async fn json_mapping_body_serializes_as_json_and_response_body_is_parsed() {
        // Echo back the request body verbatim with content-type
        // application/json so `body` (parsed) AND `body_text` (raw)
        // both reflect what the client sent.
        let app = Router::new().route(
            "/echo",
            post(|headers: HeaderMap, body: axum::body::Bytes| async move {
                let ct = headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("text/plain")
                    .to_string();
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, ct)],
                    body,
                )
            }),
        );
        let fx = start(app).await;
        let r = execute(parse_with(&format!(
            r#"
method: POST
url: "{}"
headers: {{ Content-Type: application/json }}
body: {{ hello: world }}
within: 2s
"#,
            url(&fx, "/echo")
        )))
        .await
        .expect("execute");

        assert_eq!(r.outputs.get("status").and_then(|v| v.as_u64()), Some(200));
        let parsed = r.outputs.get("body").expect("body");
        assert_eq!(parsed["hello"], serde_json::json!("world"));
        let text = r.outputs.get("body_text").and_then(|v| v.as_str()).unwrap();
        assert!(text.contains("world"));
    }

    #[tokio::test]
    async fn string_body_is_sent_verbatim_and_non_json_response_keeps_body_null() {
        // Server echoes the raw body with content-type text/plain so
        // we can verify (a) the client sent the exact string and (b)
        // `body` is null because the response isn't JSON.
        let app = Router::new().route(
            "/echo",
            post(|body: axum::body::Bytes| async move {
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/plain")],
                    body,
                )
            }),
        );
        let fx = start(app).await;
        let r = execute(parse_with(&format!(
            r#"
method: POST
url: "{}"
headers: {{ Content-Type: text/plain }}
body: "literal-string-payload"
within: 2s
"#,
            url(&fx, "/echo")
        )))
        .await
        .unwrap();

        assert_eq!(
            r.outputs.get("body_text").and_then(|v| v.as_str()),
            Some("literal-string-payload")
        );
        // Content-Type wasn't JSON, so `body` is parsed null.
        assert!(r.outputs.get("body").unwrap().is_null());
    }

    #[tokio::test]
    async fn unreachable_host_yields_http_error() {
        // Bind an ephemeral port, capture its address, then drop the
        // listener. The OS gives the port back, so a connect there
        // returns ECONNREFUSED (or an equivalent transport failure)
        // deterministically — no reliance on a "probably unused"
        // port like 1.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let r = execute(parse_with(&format!(
            r#"{{ method: GET, url: "http://{addr}/", within: 2s }}"#
        )))
        .await;
        match r {
            Err(ActionError::Http(_)) => {}
            other => panic!("expected ActionError::Http, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_string_body_mapping_key_yields_http_error() {
        // YAML mapping with an integer key isn't representable as
        // JSON; we reject explicitly rather than silently dropping
        // the entry.
        let with: With = serde_yml::from_str(
            r#"
method: POST
url: "http://127.0.0.1:0/"
body:
  1: "with-int-key"
"#,
        )
        .unwrap();
        match execute(with).await {
            Err(ActionError::Http(msg)) => {
                assert!(msg.contains("non-string mapping key"), "got: {msg}");
            }
            other => panic!("expected ActionError::Http, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn query_block_reaches_the_server_sorted_and_encoded() {
        // Server echoes back the raw query string it received so we can
        // assert the wire form: keys sorted, value urlencoded.
        let app = Router::new().route(
            "/list",
            axum::routing::get(|req: axum::http::Request<axum::body::Body>| async move {
                let q = req.uri().query().unwrap_or("").to_string();
                (StatusCode::OK, q)
            }),
        );
        let fx = start(app).await;
        let r = execute(parse_with(&format!(
            r#"
method: GET
url: "{}"
query:
  size: 10
  page: 1
  q: "a b"
within: 2s
"#,
            url(&fx, "/list")
        )))
        .await
        .expect("execute");
        assert_eq!(
            r.outputs.get("body_text").and_then(|v| v.as_str()),
            Some("page=1&q=a+b&size=10")
        );
    }

    #[tokio::test]
    async fn lowercase_method_is_normalized_to_uppercase() {
        // Server records the method it sees. We send `get`; reqwest
        // would happily pass that through as a custom extension
        // method, but normalization upgrades it to `GET` so the
        // server's standard-method dispatch matches.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let seen_get = Arc::new(AtomicBool::new(false));
        let flag = seen_get.clone();
        let app = Router::new().route(
            "/m",
            axum::routing::get(move || {
                let f = flag.clone();
                async move {
                    f.store(true, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        );
        let fx = start(app).await;
        let r = execute(parse_with(&format!(
            r#"{{ method: get, url: "{}", within: 2s }}"#,
            url(&fx, "/m")
        )))
        .await
        .unwrap();
        assert_eq!(r.outputs.get("status").and_then(|v| v.as_u64()), Some(200));
        assert!(seen_get.load(Ordering::SeqCst), "server didn't see GET");
    }

    #[tokio::test]
    async fn malformed_json_response_records_observation_and_keeps_body_null() {
        // Server claims JSON but emits garbage. `body` is `null` (no
        // valid JSON to surface) and an observation captures the
        // parse-failure signal so the trace explains the null.
        let app = Router::new().route(
            "/bad-json",
            axum::routing::get(|| async {
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    "{not valid json",
                )
            }),
        );
        let fx = start(app).await;
        let r = execute(parse_with(&format!(
            r#"{{ method: GET, url: "{}", within: 2s }}"#,
            url(&fx, "/bad-json")
        )))
        .await
        .unwrap();
        assert!(r.outputs.get("body").unwrap().is_null());
        assert!(
            r.observations
                .iter()
                .any(|o| o.kind == "api.json_parse_failure"),
            "expected json_parse_failure observation"
        );
    }

    #[tokio::test]
    async fn slow_server_past_within_yields_timeout() {
        let app = Router::new().route(
            "/slow",
            any(|| async {
                tokio::time::sleep(Duration::from_millis(500)).await;
                StatusCode::OK
            }),
        );
        let fx = start(app).await;
        let r = execute(parse_with(&format!(
            r#"{{ method: GET, url: "{}", within: 100ms }}"#,
            url(&fx, "/slow")
        )))
        .await
        .unwrap();
        assert_eq!(r.outcome, crate::action::Outcome::Timeout);
    }

    #[tokio::test]
    async fn malformed_method_yields_http_error() {
        let r = execute(parse_with(r#"{ method: "BAD METHOD", url: "http://x/" }"#)).await;
        match r {
            Err(ActionError::Http(_)) => {}
            other => panic!("expected ActionError::Http, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn five_hundred_response_is_still_outcome_ok() {
        let app = Router::new().route(
            "/boom",
            any(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "kaboom") }),
        );
        let fx = start(app).await;
        let r = execute(parse_with(&format!(
            r#"{{ method: GET, url: "{}", within: 2s }}"#,
            url(&fx, "/boom")
        )))
        .await
        .unwrap();
        // Status is data, not a verdict — `Outcome::Ok` with a 500
        // status field is the spec-confirmed shape.
        assert_eq!(r.outcome, crate::action::Outcome::Ok);
        assert_eq!(r.outputs.get("status").and_then(|v| v.as_u64()), Some(500));
    }
}
