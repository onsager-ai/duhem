//! `api/observe` — passive HTTP observation via Playwright network
//! interception.
//!
//! Spec on issue #38. Where `api/call` (#21) actively *issues* a
//! request, `api/observe` *records* one that some other step
//! triggered — typically a `ui/click` that causes the page's JS to
//! `fetch()` something. The two actions share an output shape so an
//! author can assert against `$steps.<id>.outputs.status`,
//! `$steps.<id>.outputs.response_body`, etc., regardless of how the
//! HTTP traffic was produced.
//!
//! Outputs (`response.*` mirror `api/call`'s; `request.*` are new):
//!
//! - `method`: request method (uppercased).
//! - `url`: full request URL (`https://host/path?q=1`).
//! - `request_body`: parsed JSON when the request `Content-Type`
//!   starts with `application/json`; `null` otherwise.
//! - `status`: response status code (u16 widened to integer).
//! - `response_body`: parsed JSON when the response `Content-Type`
//!   starts with `application/json`; `null` otherwise.
//! - `response_headers`: response headers as a JSON object (values
//!   rendered via UTF-8 lossy from the raw header bytes).
//! - `request_headers`: request headers as a JSON object.
//!
//! ## v1 ordering caveat
//!
//! The spec's worked example places `api/observe` *before* the
//! `ui/click` it conceptually observes. That ordering requires the
//! engine to run the observe listener concurrently with subsequent
//! steps — a Phase-1 follow-up. **v1 implementation here is
//! synchronous**: the listener subscribes when the step runs and
//! waits up to `within:` for a matching event. Authors who need the
//! observe-before-click grammar can either (a) place observe AFTER
//! the trigger and rely on Playwright's request stream still carrying
//! the in-flight or just-finished event, or (b) wait for the
//! concurrent-listener engine extension. Both choices preserve the
//! Holistic Verification Principle — no mocks at the web boundary.
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
//! - Subscription error / bad regex → `ActionError`.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use playwright::api::page::Event;
use playwright::api::{Request, Response};
use serde::Deserialize;
use tokio::time::timeout;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::error::ActionError;
use crate::with::WithinSpec;

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
    /// (the listener attaches at this step's runtime, not at the
    /// declared `after:` boundary). Reserved for the future
    /// concurrent-listener engine extension; accepted today so
    /// existing Verification Definitions don't need to migrate when
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
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "api/observe",
                source: e,
            })?;
        let timeout_dur: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);

        let matcher = UrlMatcher::parse(&with.url_pattern)?;
        let method_filter = with.method.as_deref().map(str::to_ascii_uppercase);

        let mut stream = ctx
            .page
            .subscribe_event()
            .map_err(|e| ActionError::Playwright(format!("api/observe: subscribe: {e}")))?;

        // Walk events as they arrive. We watch `Response` because
        // it's the event that carries both directions: the response
        // payload, and (via `response.request()`) the originating
        // request. `RequestFinished` would also work for the request
        // side but lacks the response body.
        let outcome = timeout(timeout_dur, async {
            loop {
                let evt_res = stream.next().await?;
                let evt = match evt_res {
                    Ok(e) => e,
                    Err(_lag) => continue, // BroadcastStream lag; skip
                };
                let resp = match evt {
                    Event::Response(r) => r,
                    _ => continue,
                };
                match match_response(&resp, &matcher, method_filter.as_deref()).await {
                    Ok(Some(outputs)) => return Some(outputs),
                    Ok(None) => continue,
                    Err(_) => continue, // Skip events that fail to introspect
                }
            }
        })
        .await;

        let outputs = match outcome {
            Ok(Some(outputs)) => outputs,
            Ok(None) | Err(_) => return Ok(ActionResult::timeout()),
        };

        let mut result = ActionResult::ok();
        for (k, v) in outputs {
            result = result.with_output(&k, v);
        }
        Ok(result)
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

/// Inspect one `Response` event, decide whether it matches the
/// observe filter, and if so collect outputs. Async because reading
/// response body/headers is async over the Playwright wire.
async fn match_response(
    resp: &Response,
    url_matcher: &UrlMatcher,
    method_filter: Option<&str>,
) -> Result<Option<BTreeMap<String, serde_json::Value>>, ActionError> {
    let url = resp
        .url()
        .map_err(|e| ActionError::Playwright(format!("api/observe: response.url: {e}")))?;
    if !url_matcher.matches(&url) {
        return Ok(None);
    }

    let req = resp.request();
    let method_norm = req
        .method()
        .map_err(|e| ActionError::Playwright(format!("api/observe: request.method: {e}")))?
        .to_ascii_uppercase();
    if let Some(want) = method_filter
        && method_norm != want
    {
        return Ok(None);
    }

    let status = resp
        .status()
        .map_err(|e| ActionError::Playwright(format!("api/observe: response.status: {e}")))?
        as u16;

    let request_headers = collect_request_headers(&req)?;
    let request_body = decode_request_body(&req, &request_headers)?;
    let response_headers = collect_response_headers(resp).await?;
    let response_body = decode_response_body(resp, &response_headers).await?;

    let mut outputs: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    outputs.insert("method".into(), serde_json::Value::String(method_norm));
    outputs.insert("url".into(), serde_json::Value::String(url));
    outputs.insert("status".into(), serde_json::Value::from(status));
    outputs.insert("request_body".into(), request_body);
    outputs.insert("response_body".into(), response_body);
    outputs.insert("request_headers".into(), headers_to_json(&request_headers));
    outputs.insert(
        "response_headers".into(),
        headers_to_json(&response_headers),
    );
    Ok(Some(outputs))
}

fn collect_request_headers(req: &Request) -> Result<BTreeMap<String, String>, ActionError> {
    let h = req
        .headers()
        .map_err(|e| ActionError::Playwright(format!("api/observe: request.headers: {e}")))?;
    // Lowercase header names for consistent lookup (HTTP is
    // case-insensitive). Preserve values verbatim.
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in h {
        out.insert(k.to_ascii_lowercase(), v);
    }
    Ok(out)
}

async fn collect_response_headers(
    resp: &Response,
) -> Result<BTreeMap<String, String>, ActionError> {
    let headers = resp
        .headers()
        .await
        .map_err(|e| ActionError::Playwright(format!("api/observe: response.headers: {e}")))?;
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for h in headers {
        out.insert(h.name.to_ascii_lowercase(), h.value);
    }
    Ok(out)
}

fn decode_request_body(
    req: &Request,
    headers: &BTreeMap<String, String>,
) -> Result<serde_json::Value, ActionError> {
    let bytes = match req
        .post_data()
        .map_err(|e| ActionError::Playwright(format!("api/observe: request.post_data: {e}")))?
    {
        Some(b) => b,
        None => return Ok(serde_json::Value::Null),
    };
    if !is_json_content_type(headers) {
        return Ok(serde_json::Value::Null);
    }
    Ok(serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

async fn decode_response_body(
    resp: &Response,
    headers: &BTreeMap<String, String>,
) -> Result<serde_json::Value, ActionError> {
    if !is_json_content_type(headers) {
        return Ok(serde_json::Value::Null);
    }
    let bytes = resp
        .body()
        .await
        .map_err(|e| ActionError::Playwright(format!("api/observe: response.body: {e}")))?;
    Ok(serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
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
}
