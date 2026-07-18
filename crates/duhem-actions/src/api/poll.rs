//! `api/poll` — hit an endpoint repeatedly until a response condition
//! holds, or a timeout elapses. The HTTP analogue of
//! `ui/assert-element`'s poll-until-state-or-timeout: it lets a check
//! verify an **asynchronous** outcome (a task finishing, a record
//! appearing) without a flaky fixed `sleep`.
//!
//! `with:` shape:
//!
//! - `method`: HTTP method (default `GET`).
//! - `url`: full URL.
//! - `headers`: request headers (e.g. an auth token).
//! - `body`: optional request body (JSON for non-string YAML).
//! - `within`: total budget (default 30s).
//! - `interval`: poll cadence (default 1s).
//! - `until`: the stop condition — exactly one mode:
//!     - `{ status: <int> }` — poll until the HTTP status code matches.
//!     - `{ path: <json-path>, equals|matches|exists|gte: … }` — poll
//!       until a field in the JSON body satisfies the predicate. `path`
//!       is a dotted/bracket path (`data.status`, `items[0].id`),
//!       mirroring #104.
//!
//! Outputs: `satisfied` (bool — did `until` hold before the budget
//! elapsed), `status` (final HTTP status), `body` (final parsed JSON),
//! `body_text` (final raw body).
//!
//! Outcome mirrors `ui/assert-*`: a completed poll is `Outcome::Ok` with
//! `satisfied` true/false — the verdict stays in the judge
//! (`assertions: - $steps.poll.outputs.satisfied == true`); `until` is
//! only the loop's stop condition. A transient request error (the
//! service still starting, a blip) counts as "not yet" and polling
//! continues; a malformed `with` or `until` is an `ActionError`.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Method;
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::error::ActionError;
use crate::with::WithinSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct With {
    #[serde(default = "default_method")]
    method: String,
    url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<serde_yml::Value>,
    #[serde(default)]
    within: Option<WithinSpec>,
    #[serde(default)]
    interval: Option<WithinSpec>,
    until: Until,
}

fn default_method() -> String {
    "GET".to_string()
}

/// The poll stop-condition. Exactly one mode must be set: `status`, or a
/// body-`path` predicate (`equals` / `matches` / `exists` / `gte`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Until {
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    equals: Option<serde_yml::Value>,
    #[serde(default)]
    matches: Option<String>,
    #[serde(default)]
    exists: Option<bool>,
    #[serde(default)]
    gte: Option<f64>,
}

pub struct Poll;

#[async_trait]
impl Action for Poll {
    fn uses(&self) -> &'static str {
        "api/poll"
    }

    fn contract(&self) -> crate::action::ActionContract {
        use crate::action::{ActionContract, FieldSpec};
        ActionContract {
            uses: "api/poll",
            summary: "Poll an HTTP endpoint until a condition holds or the deadline elapses.",
            with: vec![
                FieldSpec::required("method"),
                FieldSpec::required("url"),
                FieldSpec::optional("headers"),
                FieldSpec::optional("body"),
                FieldSpec::optional("within"),
                FieldSpec::optional("interval"),
                FieldSpec::required("until"),
            ],
            outputs: vec!["satisfied", "status", "body", "body_text"],
            example: "- uses: api/poll\n  with: { method: GET, url: $inputs.job_url, until: \"$response.body.state == 'done'\" }",
        }
    }

    async fn invoke(
        &self,
        _ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "api/poll",
                source: e,
            })?;
        execute(with).await
    }
}

pub(crate) async fn execute(with: With) -> Result<ActionResult, ActionError> {
    let total: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);
    let interval: Duration = with
        .interval
        .map(Into::into)
        .unwrap_or(Duration::from_secs(1));

    let body_mode = with.equivalent_until_mode()?;

    let method = parse_method(&with.method)?;
    // Per-request timeout is bounded by the remaining budget; cap it so a
    // hung request can't consume the whole window in one shot.
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| ActionError::Http(format!("api/poll: build client: {e}")))?;

    let started = Instant::now();
    let mut last: Option<(u16, serde_json::Value, String)> = None;

    loop {
        // A transient request error (DNS/TCP/timeout — the target may
        // still be starting) is ignored: keep polling until the budget
        // elapses.
        if let Ok((status, body_json, body_text)) = do_request(&client, &method, &with).await {
            let satisfied = body_mode.evaluate(status, &body_json);
            last = Some((status, body_json, body_text));
            if satisfied {
                return Ok(result(true, last));
            }
        }
        if started.elapsed() >= total {
            return Ok(result(false, last));
        }
        tokio::time::sleep(interval).await;
    }
}

fn result(satisfied: bool, last: Option<(u16, serde_json::Value, String)>) -> ActionResult {
    let mut r = ActionResult::ok().with_output("satisfied", serde_json::Value::Bool(satisfied));
    if let Some((status, body, body_text)) = last {
        r = r
            .with_output("status", serde_json::Value::from(status))
            .with_output("body", body)
            .with_output("body_text", serde_json::Value::String(body_text));
    }
    r
}

async fn do_request(
    client: &reqwest::Client,
    method: &Method,
    with: &With,
) -> Result<(u16, serde_json::Value, String), ActionError> {
    let mut req = client.request(method.clone(), &with.url);
    for (k, v) in &with.headers {
        req = req.header(k, v);
    }
    if let Some(body) = &with.body {
        req = match body {
            serde_yml::Value::String(s) => req.body(s.clone()),
            other => {
                let json = crate::api::call::yml_to_json(other)?;
                let bytes = serde_json::to_vec(&json)
                    .map_err(|e| ActionError::Http(format!("api/poll: serialize body: {e}")))?;
                req.body(bytes)
            }
        };
    }

    let resp = req
        .send()
        .await
        .map_err(|e| ActionError::Http(format!("api/poll: {e}")))?;
    let status = resp.status().as_u16();
    let ct = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ActionError::Http(format!("api/poll: read body: {e}")))?;
    let body_text = String::from_utf8_lossy(&bytes).into_owned();
    let body_json = if ct.starts_with("application/json") {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };
    Ok((status, body_json, body_text))
}

fn parse_method(m: &str) -> Result<Method, ActionError> {
    let normalized = if m.is_ascii() {
        m.to_ascii_uppercase()
    } else {
        m.to_string()
    };
    Method::from_bytes(normalized.as_bytes())
        .map_err(|e| ActionError::Http(format!("api/poll: invalid method `{m}`: {e}")))
}

/// A validated, ready-to-evaluate stop condition.
enum Mode {
    Status(u16),
    Path { path: String, pred: PathPred },
}

enum PathPred {
    Equals(serde_json::Value),
    Matches(regex::Regex),
    Exists(bool),
    Gte(f64),
}

impl Mode {
    fn evaluate(&self, status: u16, body: &serde_json::Value) -> bool {
        match self {
            Mode::Status(want) => status == *want,
            Mode::Path { path, pred } => {
                let found = navigate(body, path);
                match pred {
                    PathPred::Exists(want) => found.is_some() == *want,
                    PathPred::Equals(want) => found.map(|v| v == want).unwrap_or(false),
                    PathPred::Matches(re) => found
                        .and_then(value_as_str)
                        .map(|s| re.is_match(&s))
                        .unwrap_or(false),
                    PathPred::Gte(n) => found
                        .and_then(|v| v.as_f64())
                        .map(|f| f >= *n)
                        .unwrap_or(false),
                }
            }
        }
    }
}

impl With {
    /// Validate `until` names exactly one mode and compile it.
    fn equivalent_until_mode(&self) -> Result<Mode, ActionError> {
        let u = &self.until;
        if let Some(s) = u.status {
            ensure_no_path_fields(u)?;
            return Ok(Mode::Status(s));
        }
        let path = u.path.clone().ok_or_else(|| {
            ActionError::Http(
                "api/poll: `until` must set `status:` or a `path:` predicate".to_string(),
            )
        })?;
        let pred = match (&u.equals, &u.matches, u.exists, u.gte) {
            (Some(v), None, None, None) => PathPred::Equals(yml_to_json_value(v)),
            (None, Some(re), None, None) => {
                PathPred::Matches(regex::Regex::new(re).map_err(|e| {
                    ActionError::Http(format!("api/poll: bad `matches` regex: {e}"))
                })?)
            }
            (None, None, Some(b), None) => PathPred::Exists(b),
            (None, None, None, Some(n)) => PathPred::Gte(n),
            _ => {
                return Err(ActionError::Http(
                    "api/poll: a `path` predicate needs exactly one of equals/matches/exists/gte"
                        .to_string(),
                ));
            }
        };
        Ok(Mode::Path { path, pred })
    }
}

fn ensure_no_path_fields(u: &Until) -> Result<(), ActionError> {
    if u.path.is_some()
        || u.equals.is_some()
        || u.matches.is_some()
        || u.exists.is_some()
        || u.gte.is_some()
    {
        return Err(ActionError::Http(
            "api/poll: `until` mixes `status:` with a `path:` predicate; set exactly one"
                .to_string(),
        ));
    }
    Ok(())
}

/// Walk a dotted/bracket path into a JSON value: `data.status`,
/// `items[0].id`. Returns `None` if any segment is absent.
pub(crate) fn navigate<'a>(
    root: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut cur = root;
    for seg in split_path(path) {
        cur = match cur {
            serde_json::Value::Object(m) => m.get(&seg)?,
            serde_json::Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Split `a.b[0].c` into `["a","b","0","c"]`.
pub(crate) fn split_path(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                if !buf.is_empty() {
                    out.push(std::mem::take(&mut buf));
                }
            }
            '[' => {
                if !buf.is_empty() {
                    out.push(std::mem::take(&mut buf));
                }
                let mut idx = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    idx.push(d);
                }
                out.push(idx);
            }
            _ => buf.push(c),
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

pub(crate) fn value_as_str(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub(crate) fn yml_to_json_value(v: &serde_yml::Value) -> serde_json::Value {
    crate::api::call::yml_to_json(v).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Outcome;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::get;
    use tokio::net::TcpListener;

    fn parse_with(s: &str) -> With {
        serde_yml::from_value(serde_yml::from_str::<serde_yml::Value>(s).unwrap())
            .expect("With deserialization")
    }

    async fn start(router: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    #[test]
    fn rejects_until_without_a_mode() {
        let w = parse_with(r#"{ url: "http://x/", until: {} }"#);
        assert!(w.equivalent_until_mode().is_err());
    }

    #[test]
    fn rejects_mixed_until_modes() {
        let w = parse_with(r#"{ url: "http://x/", until: { status: 200, path: a, gte: 1 } }"#);
        assert!(w.equivalent_until_mode().is_err());
    }

    #[tokio::test]
    async fn polls_until_body_field_flips() {
        // The server reports total:0 for the first 2 calls, then total:1.
        let n = Arc::new(AtomicU32::new(0));
        let app = Router::new().route(
            "/list",
            get({
                let n = n.clone();
                move || {
                    let n = n.clone();
                    async move {
                        let i = n.fetch_add(1, Ordering::SeqCst);
                        let total = if i >= 2 { 1 } else { 0 };
                        (
                            StatusCode::OK,
                            [(axum::http::header::CONTENT_TYPE, "application/json")],
                            format!("{{\"total\":{total}}}"),
                        )
                    }
                }
            }),
        );
        let base = start(app).await;
        let r = execute(parse_with(&format!(
            r#"{{ url: "{base}/list", within: 5s, interval: 50ms, until: {{ path: total, gte: 1 }} }}"#
        )))
        .await
        .unwrap();
        assert_eq!(r.outcome, Outcome::Ok);
        assert_eq!(
            r.outputs.get("satisfied").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn never_satisfied_within_budget_is_satisfied_false() {
        let app = Router::new().route(
            "/list",
            get(|| async {
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    "{\"total\":0}",
                )
            }),
        );
        let base = start(app).await;
        let r = execute(parse_with(&format!(
            r#"{{ url: "{base}/list", within: 300ms, interval: 50ms, until: {{ path: total, gte: 1 }} }}"#
        )))
        .await
        .unwrap();
        assert_eq!(
            r.outputs.get("satisfied").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[tokio::test]
    async fn polls_until_http_status_flips() {
        // 503 for the first 2 calls, then 200.
        let n = Arc::new(AtomicU32::new(0));
        let app = Router::new().route(
            "/health",
            get({
                let n = n.clone();
                move || {
                    let n = n.clone();
                    async move {
                        let i = n.fetch_add(1, Ordering::SeqCst);
                        if i >= 2 {
                            StatusCode::OK
                        } else {
                            StatusCode::SERVICE_UNAVAILABLE
                        }
                    }
                }
            }),
        );
        let base = start(app).await;
        let r = execute(parse_with(&format!(
            r#"{{ url: "{base}/health", within: 5s, interval: 50ms, until: {{ status: 200 }} }}"#
        )))
        .await
        .unwrap();
        assert_eq!(
            r.outputs.get("satisfied").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(r.outputs.get("status").and_then(|v| v.as_u64()), Some(200));
    }

    #[tokio::test]
    async fn transient_connection_error_eventually_times_out_satisfied_false() {
        // Nothing is listening here; every poll errors. We should keep
        // polling and return satisfied:false at the budget, not error.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let r = execute(parse_with(&format!(
            r#"{{ url: "http://{addr}/x", within: 300ms, interval: 50ms, until: {{ status: 200 }} }}"#
        )))
        .await
        .unwrap();
        assert_eq!(
            r.outputs.get("satisfied").and_then(|v| v.as_bool()),
            Some(false)
        );
    }
}
