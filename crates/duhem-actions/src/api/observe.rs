//! `api/observe` — passive HTTP observation via the browser's network
//! events.
//!
//! Spec on issue #38; restored onto the official-Playwright sidecar
//! driver in #72 (it was temporarily stubbed in #71). Where `api/call`
//! (#21) actively *issues* a request, `api/observe` *records* one that
//! some other step triggered — typically a `ui/click` that causes the
//! page's JS to `fetch()` something. The two actions share the
//! response-side output shape (`status`, `body`, `body_text`,
//! `headers`) so an author can assert against
//! `$steps.<id>.outputs.status` or `$steps.<id>.outputs.body`
//! regardless of how the HTTP traffic was produced.
//!
//! Outputs (response side matches `api/call`'s names; `request_*`
//! are new):
//!
//! - `method`: request method (uppercased).
//! - `url`: full request URL (`https://host/path?q=1`).
//! - `request_body`: parsed JSON when the request `Content-Type`
//!   starts with `application/json`; `null` otherwise.
//! - `request_headers`: request headers as a JSON object of strings.
//! - `status`: response status code (u16 widened to integer).
//! - `body`: parsed JSON when the response `Content-Type` starts with
//!   `application/json`; `null` otherwise. Matches `api/call`.
//! - `body_text`: raw response bytes as UTF-8 (lossy). Matches
//!   `api/call`.
//! - `headers`: response headers as a JSON object of strings. Matches
//!   `api/call`. Header values are surfaced as `String` — there is no
//!   byte-fidelity path here, unlike `api/call`'s `reqwest`-backed
//!   UTF-8-lossy rendering.
//!
//! When the request or response declares `application/json` but the
//! body fails to parse, the output value stays `null` and an
//! `api.json_parse_failure` observation is appended to the action
//! result. Same shape as `api/call`'s parse-failure signal.
//!
//! ## How events reach here (#72 sidecar model)
//!
//! The sidecar attaches a `page.on('response', …)` recorder when the
//! page is created and buffers every response (body read eagerly,
//! base64-encoded). `invoke` polls that buffer via
//! [`Page::poll_network`] within `within:`, applying the URL/method
//! filter to each event and collecting the first match. Because the
//! page is created per check, the buffer only ever holds *this check's*
//! traffic — so observe scans exactly the HTTP its own check produced.
//!
//! This is strictly more robust than the pre-#71 live-stream
//! subscription: a response that finished between the triggering step
//! and this one is still in the buffer (the old broadcast subscription
//! could miss it — the "just-finished event" race the original v1
//! caveat warned about). Outputs are byte-for-byte identical; only the
//! delivery channel changed.
//!
//! ## v1 ordering caveat
//!
//! The spec's worked example places `api/observe` *before* the
//! `ui/click` it conceptually observes. That ordering requires the
//! engine to run the observe listener concurrently with subsequent
//! steps — a Phase-1 follow-up. **v1 here is synchronous**: observe
//! runs at its own step and waits up to `within:` for a matching
//! event to appear in the buffer. Authors place observe AFTER the
//! trigger; the per-page recorder guarantees a just-finished event is
//! still observable. Both choices preserve the Holistic Verification
//! Principle — no mocks at the web boundary.
//!
//! ## `url_pattern` grammar
//!
//! Default flavor matches the full URL exactly
//! (`https://host/path?q=1`). Authors building from
//! `$inputs.base_url + "/projects"` get the natural prefix match they
//! wrote, since `==` is exact — a path with a query string does NOT
//! match `https://host/projects`; v1 explicitly does not implement
//! glob semantics here.
//!
//! `re:<regex>` flavor: regex match via `regex::Regex::is_match`
//! (substring match — not anchored). Authors who want anchoring
//! write `re:^...$` themselves.
//!
//! ## Method matching
//!
//! `method:` is uppercased before comparison so authors who write
//! `method: get` still match `GET`. Omitting `method:` matches any
//! method.
//!
//! ## Outcomes
//!
//! - Matching event arrives within `within:` → `Outcome::Ok`.
//! - No matching event within `within:` → `Outcome::Timeout`.
//! - Subscription/poll error, bad regex, or a body-read failure on
//!   the matched event → `ActionError`.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use tokio::time::{sleep, timeout};

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN, Observation};
use crate::browser::NetworkEvent;
use crate::error::ActionError;
use crate::with::WithinSpec;

/// How often to re-poll the sidecar's network buffer while waiting for
/// a match. Small enough that observe resolves promptly once the event
/// lands, large enough to keep the request/response channel quiet.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct With {
    /// Optional method filter. Omitted = match any method.
    #[serde(default)]
    method: Option<String>,
    /// URL pattern. Exact string match by default; regex when
    /// prefixed `re:`. Spec on #38 § "Path-only vs. full-URL
    /// matching": v1 matches the full URL.
    url_pattern: String,
    /// Optional step-id reference. v1 does not enforce ordering
    /// (observe scans the page's recorded traffic at this step's
    /// runtime, not at the declared `after:` boundary). Reserved for
    /// the future concurrent-listener engine extension; accepted today
    /// so existing Verification Definitions don't need to migrate when
    /// it lands.
    #[serde(default)]
    #[allow(dead_code)]
    after: Option<String>,
    /// Max wait for a matching event. Defaults to [`DEFAULT_WITHIN`].
    #[serde(default)]
    within: Option<WithinSpec>,
}

pub struct Observe;

#[async_trait]
impl Action for Observe {
    fn uses(&self) -> &'static str {
        "api/observe"
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let page = ctx.require_page()?;
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "api/observe",
                source: e,
            })?;
        let timeout_dur: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);

        let matcher = UrlMatcher::parse(&with.url_pattern)?;
        let method_filter = with.method.as_deref().map(str::to_ascii_uppercase);

        // Poll the sidecar's per-page recorder until a matching event
        // appears or `within:` elapses. `cursor` advances past events
        // we've already inspected, so each one is filtered once.
        //
        // Two-phase matching: the cheap URL/method check filters first;
        // only on a confirmed match do we decode bodies (which can fail
        // — a matched event whose body failed to read surfaces as an
        // `ActionError`, never as a misleading `Outcome::Timeout`).
        let outcome = timeout(timeout_dur, async {
            let mut cursor: u64 = 0;
            loop {
                let batch = match page.poll_network(cursor).await {
                    Ok(b) => b,
                    Err(e) => {
                        return Some(Err(ActionError::Playwright(format!(
                            "api/observe: pollNetwork: {e}"
                        ))));
                    }
                };
                cursor = batch.cursor;
                for evt in &batch.events {
                    if let Some(m) = filter_event(evt, &matcher, method_filter.as_deref()) {
                        return Some(collect_match(evt, m));
                    }
                }
                sleep(POLL_INTERVAL).await;
            }
        })
        .await;

        let matched = match outcome {
            Ok(Some(Ok(matched))) => matched,
            Ok(Some(Err(e))) => return Err(e),
            Ok(None) | Err(_) => return Ok(ActionResult::timeout()),
        };

        let mut result = ActionResult::ok();
        for (k, v) in matched.outputs {
            result = result.with_output(&k, v);
        }
        result.observations.extend(matched.observations);
        Ok(result)
    }
}

/// Result of a successful `(url, method)` filter match. The outputs
/// and observations are collected separately in [`collect_match`] so
/// the latter can surface JSON-parse failures as a structured
/// observation rather than silently coercing to `null`.
#[derive(Debug)]
struct MatchedEvent {
    outputs: BTreeMap<String, serde_json::Value>,
    observations: Vec<Observation>,
}

/// The cheap-filter half of the match: read URL and method off the
/// recorded event, check both filters, return the normalized values on
/// match. Both fields are already materialized by the sidecar recorder,
/// so — unlike the pre-#71 live `Response` — there are no fallible
/// accessors here and the filter is infallible.
struct FilterMatch {
    method_norm: String,
    url: String,
}

fn filter_event(
    evt: &NetworkEvent,
    url_matcher: &UrlMatcher,
    method_filter: Option<&str>,
) -> Option<FilterMatch> {
    if !url_matcher.matches(&evt.url) {
        return None;
    }
    let method_norm = evt.method.to_ascii_uppercase();
    if let Some(want) = method_filter
        && method_norm != want
    {
        return None;
    }
    Some(FilterMatch {
        method_norm,
        url: evt.url.clone(),
    })
}

/// The heavy half of the match: decode bodies/headers and produce
/// outputs + observations. Called only after [`filter_event`] confirmed
/// the URL/method filter passes — so a body-read failure recorded on
/// the matched event (`NetworkEvent::body_error`) is real and
/// propagates as `ActionError`, mirroring the pre-#71
/// `response.body().await?` propagation.
fn collect_match(evt: &NetworkEvent, m: FilterMatch) -> Result<MatchedEvent, ActionError> {
    let FilterMatch { method_norm, url } = m;

    let status = evt.status;
    let request_headers = lowercase_headers(&evt.request_headers);
    let response_headers = lowercase_headers(&evt.response_headers);

    let body_bytes = decode_response_bytes(evt)?;

    let mut observations: Vec<Observation> = Vec::new();
    let request_body = decode_request_body(evt, &request_headers, &mut observations)?;
    let (response_body, body_text) =
        decode_response_body(&body_bytes, &response_headers, &mut observations);

    let mut outputs: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    outputs.insert("method".into(), serde_json::Value::String(method_norm));
    outputs.insert("url".into(), serde_json::Value::String(url));
    outputs.insert("status".into(), serde_json::Value::from(status));
    outputs.insert("request_body".into(), request_body);
    outputs.insert("request_headers".into(), headers_to_json(&request_headers));
    // Response-side output names mirror `api/call`'s so authors can
    // write assertions like `$steps.x.outputs.status == 201` and
    // `$steps.x.outputs.body.id == "..."` regardless of whether `x`
    // was an `api/call` or `api/observe` step.
    outputs.insert("body".into(), response_body);
    outputs.insert("body_text".into(), serde_json::Value::String(body_text));
    outputs.insert("headers".into(), headers_to_json(&response_headers));
    Ok(MatchedEvent {
        outputs,
        observations,
    })
}

/// Decode the matched event's base64 response body to raw bytes. A
/// recorded body-read failure (`body_error`) propagates here — this is
/// the collect-on-match propagation point. An empty body decodes to an
/// empty slice (the sidecar sends `""`, not `null`, for a 0-byte body).
fn decode_response_bytes(evt: &NetworkEvent) -> Result<Vec<u8>, ActionError> {
    match &evt.body_base64 {
        Some(b64) => BASE64.decode(b64).map_err(|e| {
            ActionError::Playwright(format!("api/observe: response.body base64 decode: {e}"))
        }),
        None => {
            let why = evt
                .body_error
                .as_deref()
                .unwrap_or("response body unavailable");
            Err(ActionError::Playwright(format!(
                "api/observe: response.body: {why}"
            )))
        }
    }
}

/// Wrapper around the two `url_pattern` flavors. Parsed once at
/// invocation time so we don't re-compile the regex on every event.
enum UrlMatcher {
    Exact(String),
    Regex(regex::Regex),
}

impl UrlMatcher {
    fn parse(pattern: &str) -> Result<Self, ActionError> {
        if let Some(re_body) = pattern.strip_prefix("re:") {
            let re = regex::Regex::new(re_body).map_err(|e| ActionError::InvalidWith {
                action: "api/observe",
                source: serde_yml::Error::custom(format!(
                    "invalid regex in url_pattern `{pattern}`: {e}"
                )),
            })?;
            Ok(UrlMatcher::Regex(re))
        } else {
            Ok(UrlMatcher::Exact(pattern.to_string()))
        }
    }

    fn matches(&self, url: &str) -> bool {
        match self {
            UrlMatcher::Exact(p) => p == url,
            UrlMatcher::Regex(r) => r.is_match(url),
        }
    }
}

/// Lowercase header names for consistent lookup (HTTP is
/// case-insensitive); preserve values verbatim. The sidecar already
/// lowercases via Playwright, but this keeps the contract local and
/// idempotent.
fn lowercase_headers(h: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    h.iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
        .collect()
}

fn decode_request_body(
    evt: &NetworkEvent,
    headers: &BTreeMap<String, String>,
    observations: &mut Vec<Observation>,
) -> Result<serde_json::Value, ActionError> {
    let bytes = match &evt.request_body_base64 {
        Some(b64) => BASE64.decode(b64).map_err(|e| {
            ActionError::Playwright(format!("api/observe: request.body base64 decode: {e}"))
        })?,
        None => return Ok(serde_json::Value::Null),
    };
    if !is_json_content_type(headers) {
        return Ok(serde_json::Value::Null);
    }
    match serde_json::from_slice(&bytes) {
        Ok(v) => Ok(v),
        Err(e) => {
            // Mirror `api/call`'s parse-failure observation so authors
            // can distinguish "no JSON body" from "declared JSON but
            // unparseable" in the trace.
            observations.push(Observation {
                kind: "api.json_parse_failure".to_string(),
                note: Some(format!(
                    "request body declared application/json but failed to parse: {e}"
                )),
            });
            Ok(serde_json::Value::Null)
        }
    }
}

/// Returns `(body_as_json_or_null, body_text)`. `body_text` is the
/// raw response bytes rendered as UTF-8 lossy — same shape as
/// `api/call`. JSON parse failures on a `Content-Type:
/// application/json` body surface as an `api.json_parse_failure`
/// observation. Bytes are already decoded (see [`decode_response_bytes`]),
/// so this step is infallible.
fn decode_response_body(
    bytes: &[u8],
    headers: &BTreeMap<String, String>,
    observations: &mut Vec<Observation>,
) -> (serde_json::Value, String) {
    let body_text = String::from_utf8_lossy(bytes).into_owned();
    if !is_json_content_type(headers) {
        return (serde_json::Value::Null, body_text);
    }
    let parsed = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => {
            observations.push(Observation {
                kind: "api.json_parse_failure".to_string(),
                note: Some(format!(
                    "response body declared application/json but failed to parse: {e}"
                )),
            });
            serde_json::Value::Null
        }
    };
    (parsed, body_text)
}

fn is_json_content_type(headers: &BTreeMap<String, String>) -> bool {
    headers
        .get("content-type")
        .map(|s| s.to_ascii_lowercase().starts_with("application/json"))
        .unwrap_or(false)
}

fn headers_to_json(h: &BTreeMap<String, String>) -> serde_json::Value {
    serde_json::Value::Object(
        h.iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect(),
    )
}

// Thin shim so the macro `serde_yml::Error::custom` path resolves;
// `serde::de::Error::custom` is the trait method.
use serde::de::Error as SerdeError;

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(s: &str) -> serde_yml::Value {
        serde_yml::from_str(s).unwrap()
    }

    /// Build a minimal `NetworkEvent` for collect/filter tests.
    fn event(method: &str, url: &str) -> NetworkEvent {
        NetworkEvent {
            method: method.to_string(),
            url: url.to_string(),
            status: 200,
            request_headers: BTreeMap::new(),
            request_body_base64: None,
            response_headers: BTreeMap::new(),
            body_base64: Some(String::new()),
            body_error: None,
        }
    }

    fn b64(s: &str) -> String {
        BASE64.encode(s.as_bytes())
    }

    #[test]
    fn parses_full_with() {
        let w: With = serde_yml::from_value(yaml(
            r#"
method: POST
url_pattern: "http://x/projects"
after: nav
within: 3s
"#,
        ))
        .unwrap();
        assert_eq!(w.method.as_deref(), Some("POST"));
        assert_eq!(w.url_pattern, "http://x/projects");
        assert_eq!(w.after.as_deref(), Some("nav"));
        let d: Duration = w.within.unwrap().into();
        assert_eq!(d, Duration::from_secs(3));
    }

    #[test]
    fn parses_minimal_with_url_pattern_only() {
        let w: With = serde_yml::from_value(yaml(r#"{ url_pattern: "/x" }"#)).unwrap();
        assert!(w.method.is_none());
        assert!(w.after.is_none());
        assert!(w.within.is_none());
        assert_eq!(w.url_pattern, "/x");
    }

    #[test]
    fn rejects_unknown_field() {
        let r: Result<With, _> = serde_yml::from_str(r#"{ url_pattern: "/x", color: red }"#);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_missing_url_pattern() {
        // Spec on #38 § Test: "rejects both empty (`method` +
        // `url_pattern` both omitted) because that matches everything
        // and is almost always a mistake." `url_pattern` is the
        // required field; omitting it surfaces as a serde error.
        let r: Result<With, _> = serde_yml::from_str(r#"{ method: GET }"#);
        assert!(r.is_err());
    }

    #[test]
    fn exact_url_matcher_matches_literally() {
        let m = UrlMatcher::parse("http://host/path").unwrap();
        assert!(m.matches("http://host/path"));
        assert!(!m.matches("http://host/path?q=1"));
        assert!(!m.matches("http://host/other"));
    }

    #[test]
    fn regex_url_matcher_handles_re_prefix() {
        let m = UrlMatcher::parse("re:^/projects/[a-f0-9-]+$").unwrap();
        assert!(m.matches("/projects/abc-123"));
        assert!(!m.matches("/projects/"));
        assert!(!m.matches("/projects/abc?q=1"));
    }

    #[test]
    fn malformed_regex_rejects_at_parse() {
        let r = UrlMatcher::parse("re:[");
        assert!(matches!(r, Err(ActionError::InvalidWith { .. })));
    }

    #[test]
    fn json_content_type_recognized_with_charset_param() {
        let mut h = BTreeMap::new();
        h.insert(
            "content-type".to_string(),
            "application/json; charset=utf-8".to_string(),
        );
        assert!(is_json_content_type(&h));
    }

    #[test]
    fn non_json_content_type_returns_false() {
        let mut h = BTreeMap::new();
        h.insert("content-type".to_string(), "text/html".to_string());
        assert!(!is_json_content_type(&h));
    }

    #[test]
    fn header_lookup_is_case_insensitive_via_lowercased_keys() {
        // Sanity for `is_json_content_type`: header names are
        // lowercased on collection so authors don't have to know
        // whether the server returned `Content-Type` or
        // `content-type`.
        let mut h = BTreeMap::new();
        h.insert("content-type".to_string(), "application/json".to_string());
        assert!(is_json_content_type(&h));
    }

    #[test]
    fn headers_serialize_as_object_of_strings() {
        let mut h = BTreeMap::new();
        h.insert("content-type".to_string(), "application/json".to_string());
        h.insert("x-trace".to_string(), "abc".to_string());
        let j = headers_to_json(&h);
        assert_eq!(j["content-type"], serde_json::json!("application/json"));
        assert_eq!(j["x-trace"], serde_json::json!("abc"));
    }

    #[test]
    fn lowercase_headers_normalizes_keys() {
        let mut h = BTreeMap::new();
        h.insert("Content-Type".to_string(), "application/json".to_string());
        h.insert("X-Trace".to_string(), "abc".to_string());
        let lower = lowercase_headers(&h);
        assert_eq!(
            lower.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(lower.get("x-trace").map(String::as_str), Some("abc"));
    }

    #[test]
    fn filter_event_matches_url_and_method() {
        let matcher = UrlMatcher::parse("http://host/api/projects").unwrap();
        let evt = event("post", "http://host/api/projects");
        let m = filter_event(&evt, &matcher, Some("POST")).expect("should match");
        // method is uppercased on match
        assert_eq!(m.method_norm, "POST");
        assert_eq!(m.url, "http://host/api/projects");
    }

    #[test]
    fn filter_event_rejects_method_mismatch() {
        let matcher = UrlMatcher::parse("http://host/api/projects").unwrap();
        let evt = event("GET", "http://host/api/projects");
        assert!(filter_event(&evt, &matcher, Some("POST")).is_none());
    }

    #[test]
    fn filter_event_no_method_filter_matches_any() {
        let matcher = UrlMatcher::parse("http://host/api/projects").unwrap();
        let evt = event("DELETE", "http://host/api/projects");
        assert!(filter_event(&evt, &matcher, None).is_some());
    }

    #[test]
    fn collect_match_decodes_json_response_body() {
        let mut evt = event("POST", "http://host/api/projects");
        evt.status = 201;
        evt.response_headers
            .insert("content-type".to_string(), "application/json".to_string());
        evt.body_base64 = Some(b64(r#"{"id":"p1","name":"Demo"}"#));
        let m = FilterMatch {
            method_norm: "POST".to_string(),
            url: evt.url.clone(),
        };
        let matched = collect_match(&evt, m).unwrap();
        assert_eq!(matched.outputs["status"], serde_json::json!(201));
        assert_eq!(matched.outputs["body"]["id"], serde_json::json!("p1"));
        assert_eq!(
            matched.outputs["body_text"],
            serde_json::json!(r#"{"id":"p1","name":"Demo"}"#)
        );
        assert!(matched.observations.is_empty());
    }

    #[test]
    fn collect_match_decodes_json_request_body() {
        let mut evt = event("POST", "http://host/api/projects");
        evt.request_headers
            .insert("content-type".to_string(), "application/json".to_string());
        evt.request_body_base64 = Some(b64(r#"{"name":"Demo"}"#));
        let m = FilterMatch {
            method_norm: "POST".to_string(),
            url: evt.url.clone(),
        };
        let matched = collect_match(&evt, m).unwrap();
        assert_eq!(
            matched.outputs["request_body"]["name"],
            serde_json::json!("Demo")
        );
    }

    #[test]
    fn collect_match_non_json_body_is_null_with_text_preserved() {
        let mut evt = event("GET", "http://host/page");
        evt.response_headers
            .insert("content-type".to_string(), "text/html".to_string());
        evt.body_base64 = Some(b64("<html></html>"));
        let m = FilterMatch {
            method_norm: "GET".to_string(),
            url: evt.url.clone(),
        };
        let matched = collect_match(&evt, m).unwrap();
        assert_eq!(matched.outputs["body"], serde_json::Value::Null);
        assert_eq!(
            matched.outputs["body_text"],
            serde_json::json!("<html></html>")
        );
    }

    #[test]
    fn collect_match_malformed_json_emits_parse_failure_observation() {
        let mut evt = event("GET", "http://host/api/x");
        evt.response_headers
            .insert("content-type".to_string(), "application/json".to_string());
        evt.body_base64 = Some(b64("{not json"));
        let m = FilterMatch {
            method_norm: "GET".to_string(),
            url: evt.url.clone(),
        };
        let matched = collect_match(&evt, m).unwrap();
        assert_eq!(matched.outputs["body"], serde_json::Value::Null);
        assert_eq!(matched.observations.len(), 1);
        assert_eq!(matched.observations[0].kind, "api.json_parse_failure");
    }

    #[test]
    fn collect_match_body_error_propagates_as_action_error() {
        let mut evt = event("GET", "http://host/api/x");
        evt.body_base64 = None;
        evt.body_error = Some("net::ERR_ABORTED".to_string());
        let m = FilterMatch {
            method_norm: "GET".to_string(),
            url: evt.url.clone(),
        };
        let err = collect_match(&evt, m).unwrap_err();
        assert!(matches!(err, ActionError::Playwright(_)));
        assert!(err.to_string().contains("net::ERR_ABORTED"));
    }
}
