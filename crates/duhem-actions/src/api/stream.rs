//! `api/stream` — follow an open-ended Server-Sent-Events (SSE) /
//! chunked `text/event-stream` from an in-progress source, collecting
//! events until a terminal condition or the `within:` budget, then
//! expose them as outputs for mechanical assertion.
//!
//! This is the live-follow analogue of `api/call`: where `api/call`
//! reads one finished response body, `api/stream` *subscribes* to a
//! stream and parses the SSE framing (`event:` / `data:` lines,
//! multi-line `data:`, `\n\n` event separators) incrementally as bytes
//! arrive, so an author can assert on the **ordered** sequence of
//! events as they were emitted — not just a finished replay. It is the
//! gap the dashboard regression VD (#152, #153) hit: today only the
//! finished-replay body is assertable via `api/call`.
//!
//! Page-free (no Playwright) — registered like `api/call` / `api/poll`
//! (`uses_requires_page` returns `false`).
//!
//! `with:` shape (mirrors `api/call` + `api/poll`):
//!
//! - `url`: full URL of the stream.
//! - `method`: HTTP method (default `GET`).
//! - `headers`: request headers (e.g. an auth token).
//! - `body`: optional request body (JSON for non-string YAML).
//! - `within`: wall-clock collection budget (default [`DEFAULT_WITHIN`]).
//!   Reaching it **ends collection** and is *not* a failure by itself —
//!   the events gathered so far are surfaced and the outcome is
//!   `Outcome::Ok`.
//! - terminal condition (optional, may combine):
//!     - `until_event: <name>` — stop once an SSE event whose `event:`
//!       field equals `<name>` arrives (that event is included).
//!     - `max_events: <n>` — stop once `n` events have been collected.
//!     - the server closing the stream (EOF) is always a terminal too.
//!
//! Closed and deterministic-by-timeout: no LLM, no scripting. The only
//! ways collection ends are `until_event`, `max_events`, the server
//! closing the stream, or the `within:` budget — every one mechanical.
//!
//! Outputs (fixed schema):
//!
//! - `status`: the stream response's HTTP status code (integer), so a
//!   check can confirm the subscription itself was accepted (`== 200`)
//!   independently of what the stream then carried. Mirrors
//!   `api/call`'s `status`.
//! - `events`: ordered JSON array of `{ event, data, data_text }`, one
//!   per SSE event in arrival order. `data` is the event's `data:`
//!   payload parsed as JSON when it parses (so nested navigation works
//!   like `api/call`'s `body.*`, #104), `null` otherwise; `data_text`
//!   is always the raw payload string. `event` is the SSE `event:`
//!   field (defaulting to `"message"` per the SSE spec when absent).
//! - `event_count`: number of collected events (integer).
//! - `last_event`: the final element of `events` (same shape), or
//!   `null` when no event arrived. `last_event.data.*` navigates into
//!   the terminal event's parsed JSON.
//! - `stopped_reason`: why collection ended — `"until_event"`,
//!   `"max_events"`, `"stream_end"`, `"timeout"`, or `"stream_error"`.
//!
//! Outcome mapping:
//!
//! - Collection ends by any terminal (including `within:`) → `Outcome::Ok`.
//!   Like `api/call`'s status, the *contents* of the stream are data,
//!   not a verdict — the assertion is where the ordered events and the
//!   terminal event get judged.
//! - The initial connection failing (DNS / TCP / TLS / malformed
//!   method or URL) → `ActionError::Http` → `Outcome::Error`. A
//!   transport drop *after* events have been read stops collection and
//!   returns `Outcome::Ok` with a `stream_error` observation, so a
//!   mid-stream blip doesn't erase the events already asserted-upon.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Method;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN, Observation};
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
    /// Stop once an SSE event with this `event:` name arrives.
    #[serde(default)]
    until_event: Option<String>,
    /// Stop once this many events have been collected.
    #[serde(default)]
    max_events: Option<usize>,
}

fn default_method() -> String {
    "GET".to_string()
}

pub struct Stream;

#[async_trait]
impl Action for Stream {
    fn uses(&self) -> &'static str {
        "api/stream"
    }

    fn contract(&self) -> crate::action::ActionContract {
        use crate::action::{ActionContract, FieldSpec};
        ActionContract {
            uses: "api/stream",
            summary: "Consume a streaming (SSE) endpoint, collecting events until a condition.",
            with: vec![
                FieldSpec::required("method"),
                FieldSpec::required("url"),
                FieldSpec::optional("headers"),
                FieldSpec::optional("body"),
                FieldSpec::optional("within"),
                FieldSpec::optional("until_event"),
                FieldSpec::optional("max_events"),
            ],
            outputs: vec!["status", "events", "event_count", "last_event"],
            example: "- uses: api/stream\n  with: { method: GET, url: $inputs.events_url, until_event: done }\n  outputs: { count: event_count }",
        }
    }

    async fn invoke(
        &self,
        _ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "api/stream",
                source: e,
            })?;
        execute(with).await
    }
}

/// Why the collection loop stopped. Surfaced as the `stopped_reason`
/// output so an author can distinguish a clean terminal from a budget
/// cut-off mechanically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    UntilEvent,
    MaxEvents,
    StreamEnd,
    Timeout,
    StreamError,
}

impl StopReason {
    fn as_str(self) -> &'static str {
        match self {
            StopReason::UntilEvent => "until_event",
            StopReason::MaxEvents => "max_events",
            StopReason::StreamEnd => "stream_end",
            StopReason::Timeout => "timeout",
            StopReason::StreamError => "stream_error",
        }
    }
}

/// Opens the stream and collects events. Factored out from
/// `Action::invoke` so the network behavior can be unit-tested without
/// constructing a Playwright `Page`.
pub(crate) async fn execute(with: With) -> Result<ActionResult, ActionError> {
    let budget: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);
    let method = parse_method(&with.method)?;

    // No client-level `.timeout()`: that caps the whole request and
    // would abort a long-lived stream. The `within:` budget is enforced
    // per-chunk via `tokio::time::timeout` against a fixed deadline.
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| ActionError::Http(format!("api/stream: build client: {e}")))?;

    let mut req = client.request(method, &with.url);
    for (k, v) in &with.headers {
        req = req.header(k, v);
    }
    if let Some(body) = &with.body {
        req = match body {
            serde_yml::Value::String(s) => req.body(s.clone()),
            other => {
                let json = crate::api::call::yml_to_json(other)?;
                let bytes = serde_json::to_vec(&json)
                    .map_err(|e| ActionError::Http(format!("api/stream: serialize body: {e}")))?;
                req.body(bytes)
            }
        };
    }

    let deadline = Instant::now() + budget;
    // The connection handshake counts against the budget too.
    let mut resp = match tokio::time::timeout_at(deadline.into(), req.send()).await {
        Err(_elapsed) => return Ok(finalize(None, Vec::new(), StopReason::Timeout, None)),
        Ok(Err(e)) => return Err(ActionError::Http(format!("api/stream: {e}"))),
        Ok(Ok(r)) => r,
    };
    let status = resp.status().as_u16();

    let mut parser = SseParser::new();
    let mut events: Vec<RawEvent> = Vec::new();
    let mut error_note: Option<String> = None;

    let stop = 'collect: loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break StopReason::Timeout;
        }
        match tokio::time::timeout(remaining, resp.chunk()).await {
            Err(_elapsed) => break StopReason::Timeout,
            Ok(Err(e)) => {
                error_note = Some(format!("transport error mid-stream: {e}"));
                break StopReason::StreamError;
            }
            Ok(Ok(None)) => {
                // Server closed the stream. Flush any trailing event
                // that wasn't terminated by a blank line.
                for ev in parser.finish() {
                    events.push(ev);
                }
                break StopReason::StreamEnd;
            }
            Ok(Ok(Some(bytes))) => {
                let chunk = String::from_utf8_lossy(&bytes);
                for ev in parser.feed(&chunk) {
                    let matched_until = with
                        .until_event
                        .as_deref()
                        .is_some_and(|name| ev.event == name);
                    events.push(ev);
                    if matched_until {
                        break 'collect StopReason::UntilEvent;
                    }
                    if with.max_events.is_some_and(|n| events.len() >= n) {
                        break 'collect StopReason::MaxEvents;
                    }
                }
            }
        }
    };

    Ok(finalize(Some(status), events, stop, error_note))
}

/// One SSE event in arrival order: the `event:` name and the joined
/// `data:` payload.
#[derive(Debug, Clone, PartialEq)]
struct RawEvent {
    event: String,
    data: String,
}

/// Build the `ActionResult` from the collected events. `data` is parsed
/// as JSON when possible (mirroring `api/call`'s `body`); `data_text`
/// keeps the raw payload (mirroring `body_text`).
fn finalize(
    status: Option<u16>,
    events: Vec<RawEvent>,
    stop: StopReason,
    error_note: Option<String>,
) -> ActionResult {
    let json_events: Vec<serde_json::Value> = events.iter().map(event_to_json).collect();
    let last = json_events
        .last()
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let mut result = ActionResult::ok();
    if let Some(code) = status {
        result = result.with_output("status", serde_json::Value::from(code));
    }
    let mut result = result
        .with_output("events", serde_json::Value::Array(json_events.clone()))
        .with_output("event_count", serde_json::Value::from(json_events.len()))
        .with_output("last_event", last)
        .with_output(
            "stopped_reason",
            serde_json::Value::String(stop.as_str().to_string()),
        );
    if let Some(note) = error_note {
        result.observations.push(Observation {
            kind: "api.stream_error".to_string(),
            note: Some(note),
        });
    }
    result
}

fn event_to_json(ev: &RawEvent) -> serde_json::Value {
    let data_json =
        serde_json::from_str::<serde_json::Value>(&ev.data).unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "event": ev.event,
        "data": data_json,
        "data_text": ev.data,
    })
}

fn parse_method(m: &str) -> Result<Method, ActionError> {
    let normalized = if m.is_ascii() {
        m.to_ascii_uppercase()
    } else {
        m.to_string()
    };
    Method::from_bytes(normalized.as_bytes())
        .map_err(|e| ActionError::Http(format!("api/stream: invalid method `{m}`: {e}")))
}

/// Incremental Server-Sent-Events framing parser.
///
/// Feeds arbitrary byte-chunks and emits complete events as their
/// terminating blank line arrives. Implements the wire rules the action
/// relies on: `event:` / `data:` fields, multi-line `data:` joined with
/// `\n`, one optional leading space stripped after the colon, comment
/// lines (`:` prefix) ignored, and `\r\n` / `\n` line endings. An event
/// is dispatched on a blank line; [`SseParser::finish`] flushes a final
/// event a server may have left unterminated at EOF.
struct SseParser {
    /// Bytes received but not yet split into a complete line.
    line_buf: String,
    /// The current event's `event:` field, if any has been seen.
    event_name: Option<String>,
    /// The current event's accumulated `data:` lines.
    data_lines: Vec<String>,
    /// Whether the current event block has seen any field at all (so a
    /// stray blank line between events doesn't emit an empty event).
    started: bool,
}

impl SseParser {
    fn new() -> Self {
        Self {
            line_buf: String::new(),
            event_name: None,
            data_lines: Vec::new(),
            started: false,
        }
    }

    /// Feed a chunk of stream text, returning any events completed by it.
    fn feed(&mut self, chunk: &str) -> Vec<RawEvent> {
        self.line_buf.push_str(chunk);
        let mut out = Vec::new();
        while let Some(nl) = self.line_buf.find('\n') {
            let mut line: String = self.line_buf.drain(..=nl).collect();
            line.pop(); // drop the '\n'
            if line.ends_with('\r') {
                line.pop();
            }
            if let Some(ev) = self.process_line(&line) {
                out.push(ev);
            }
        }
        out
    }

    /// Flush any event left pending at end-of-stream. A server may close
    /// without the final blank line; per robustness we still surface the
    /// event if it carried any field.
    fn finish(&mut self) -> Vec<RawEvent> {
        let mut out = Vec::new();
        if !self.line_buf.is_empty() {
            let line = std::mem::take(&mut self.line_buf);
            if let Some(ev) = self.process_line(&line) {
                out.push(ev);
            }
        }
        if let Some(ev) = self.dispatch() {
            out.push(ev);
        }
        out
    }

    /// Process one logical line. Returns an event when the line is the
    /// blank dispatch boundary.
    fn process_line(&mut self, line: &str) -> Option<RawEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            // Comment line — ignored, but keeps the block alive.
            self.started = true;
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            // A line with no colon is a field name with an empty value.
            None => (line, ""),
        };
        self.started = true;
        match field {
            "event" => self.event_name = Some(value.to_string()),
            "data" => self.data_lines.push(value.to_string()),
            // `id` / `retry` and any unknown field are accepted and
            // ignored: assertion targets are `event` + `data`.
            _ => {}
        }
        None
    }

    /// Emit the accumulated event and reset for the next block. Returns
    /// `None` for a boundary that closes an empty (never-started) block.
    fn dispatch(&mut self) -> Option<RawEvent> {
        if !self.started {
            return None;
        }
        let event = self
            .event_name
            .take()
            .unwrap_or_else(|| "message".to_string());
        let data = self.data_lines.join("\n");
        self.data_lines.clear();
        self.started = false;
        Some(RawEvent { event, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::Router;
    use axum::http::header;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use futures::StreamExt;
    use tokio::net::TcpListener;

    fn parse_with(s: &str) -> With {
        serde_yml::from_value(serde_yml::from_str::<serde_yml::Value>(s).unwrap())
            .expect("With deserialization")
    }

    // --- With deserialization ----------------------------------------

    #[test]
    fn parses_minimal_and_full_with() {
        let w = parse_with(r#"{ url: "http://x/" }"#);
        assert_eq!(w.method, "GET");
        assert!(w.until_event.is_none());
        assert!(w.max_events.is_none());

        let w = parse_with(
            r#"{ method: POST, url: "http://x/", headers: { A: b }, within: 9s, until_event: done, max_events: 4 }"#,
        );
        assert_eq!(w.method, "POST");
        assert_eq!(w.until_event.as_deref(), Some("done"));
        assert_eq!(w.max_events, Some(4));
        let d: Duration = w.within.unwrap().into();
        assert_eq!(d, Duration::from_secs(9));
    }

    #[test]
    fn rejects_unknown_field() {
        let r: Result<With, _> = serde_yml::from_str(r#"{ url: "x", color: red }"#);
        assert!(r.is_err());
    }

    // --- SSE frame parser --------------------------------------------

    #[test]
    fn parses_single_event_with_name_and_data() {
        let mut p = SseParser::new();
        let evs = p.feed("event: trace\ndata: hello\n\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].event, "trace");
        assert_eq!(evs[0].data, "hello");
    }

    #[test]
    fn joins_multi_line_data_with_newline() {
        let mut p = SseParser::new();
        let evs = p.feed("event: trace\ndata: line one\ndata: line two\n\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].data, "line one\nline two");
    }

    #[test]
    fn parses_multiple_events_in_one_chunk() {
        let mut p = SseParser::new();
        let evs = p.feed("event: a\ndata: 1\n\nevent: b\ndata: 2\n\n");
        assert_eq!(evs.len(), 2);
        assert_eq!((evs[0].event.as_str(), evs[0].data.as_str()), ("a", "1"));
        assert_eq!((evs[1].event.as_str(), evs[1].data.as_str()), ("b", "2"));
    }

    #[test]
    fn reassembles_event_split_across_chunks() {
        let mut p = SseParser::new();
        assert!(p.feed("event: tr").is_empty());
        assert!(p.feed("ace\ndata: par").is_empty());
        let evs = p.feed("tial\n\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].event, "trace");
        assert_eq!(evs[0].data, "partial");
    }

    #[test]
    fn defaults_event_name_to_message_and_handles_crlf_and_comments() {
        let mut p = SseParser::new();
        // CRLF line endings, a comment line, and no `event:` field.
        let evs = p.feed(": keep-alive\r\ndata: payload\r\n\r\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].event, "message");
        assert_eq!(evs[0].data, "payload");
    }

    #[test]
    fn finish_flushes_unterminated_trailing_event() {
        let mut p = SseParser::new();
        // No trailing blank line — server closed mid-block.
        assert!(p.feed("event: trace\ndata: last").is_empty());
        let evs = p.finish();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].event, "trace");
        assert_eq!(evs[0].data, "last");
    }

    #[test]
    fn finish_with_nothing_pending_emits_nothing() {
        let mut p = SseParser::new();
        let _ = p.feed("data: x\n\n");
        assert!(p.finish().is_empty());
    }

    // --- network behavior --------------------------------------------

    /// Serve a fixed `text/event-stream` body and then close.
    async fn serve_body(body: &'static str) -> String {
        let app = Router::new().route(
            "/live",
            get(move || async move {
                ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response()
            }),
        );
        spawn(app).await
    }

    async fn spawn(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}/live")
    }

    #[tokio::test]
    async fn collects_ordered_events_until_stream_end() {
        let url = serve_body(
            "event: trace\ndata: {\"seq\":1}\n\nevent: trace\ndata: {\"kind\":\"run_finished\",\"verdict\":\"pass\"}\n\n",
        )
        .await;
        let r = execute(parse_with(&format!(r#"{{ url: "{url}", within: 5s }}"#)))
            .await
            .unwrap();
        assert_eq!(r.outcome, crate::action::Outcome::Ok);
        assert_eq!(r.outputs.get("status").and_then(|v| v.as_u64()), Some(200));
        assert_eq!(
            r.outputs.get("event_count").and_then(|v| v.as_u64()),
            Some(2)
        );
        assert_eq!(
            r.outputs.get("stopped_reason").and_then(|v| v.as_str()),
            Some("stream_end")
        );
        // Nested navigation into the terminal event's parsed JSON.
        let last = r.outputs.get("last_event").unwrap();
        assert_eq!(last["event"], serde_json::json!("trace"));
        assert_eq!(last["data"]["verdict"], serde_json::json!("pass"));
        // Ordered: first event is seq 1.
        let events = r.outputs.get("events").unwrap().as_array().unwrap();
        assert_eq!(events[0]["data"]["seq"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn until_event_stops_at_the_named_event() {
        let url = serve_body(
            "event: tick\ndata: 1\n\nevent: done\ndata: stop\n\nevent: tick\ndata: 2\n\n",
        )
        .await;
        let r = execute(parse_with(&format!(
            r#"{{ url: "{url}", within: 5s, until_event: done }}"#
        )))
        .await
        .unwrap();
        assert_eq!(
            r.outputs.get("stopped_reason").and_then(|v| v.as_str()),
            Some("until_event")
        );
        // Stopped at `done` (2 events), not the trailing `tick`.
        assert_eq!(
            r.outputs.get("event_count").and_then(|v| v.as_u64()),
            Some(2)
        );
        assert_eq!(
            r.outputs.get("last_event").unwrap()["event"],
            serde_json::json!("done")
        );
    }

    #[tokio::test]
    async fn max_events_caps_collection() {
        let url = serve_body("data: 1\n\ndata: 2\n\ndata: 3\n\ndata: 4\n\n").await;
        let r = execute(parse_with(&format!(
            r#"{{ url: "{url}", within: 5s, max_events: 2 }}"#
        )))
        .await
        .unwrap();
        assert_eq!(
            r.outputs.get("stopped_reason").and_then(|v| v.as_str()),
            Some("max_events")
        );
        assert_eq!(
            r.outputs.get("event_count").and_then(|v| v.as_u64()),
            Some(2)
        );
    }

    #[tokio::test]
    async fn within_budget_ends_collection_without_failing() {
        // The server emits one event, then holds the connection open
        // (never closing, never sending more). The `within:` budget must
        // end collection with the event captured and `Outcome::Ok`.
        let app = Router::new().route(
            "/live",
            get(|| async {
                let stream = futures::stream::once(async {
                    Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(
                        "event: trace\ndata: only\n\n",
                    ))
                })
                .chain(futures::stream::pending());
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    axum::body::Body::from_stream(stream),
                )
                    .into_response()
            }),
        );
        let url = spawn(app).await;
        let started = Instant::now();
        let r = execute(parse_with(&format!(r#"{{ url: "{url}", within: 300ms }}"#)))
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(3), "budget bounded");
        assert_eq!(r.outcome, crate::action::Outcome::Ok);
        assert_eq!(
            r.outputs.get("stopped_reason").and_then(|v| v.as_str()),
            Some("timeout")
        );
        assert_eq!(
            r.outputs.get("event_count").and_then(|v| v.as_u64()),
            Some(1)
        );
        assert_eq!(
            r.outputs.get("last_event").unwrap()["data_text"],
            serde_json::json!("only")
        );
    }

    #[tokio::test]
    async fn unreachable_host_yields_http_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let r = execute(parse_with(&format!(
            r#"{{ url: "http://{addr}/live", within: 2s }}"#
        )))
        .await;
        match r {
            Err(ActionError::Http(_)) => {}
            other => panic!("expected ActionError::Http, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_json_data_is_null_data_with_raw_text_preserved() {
        let url = serve_body("event: note\ndata: not json\n\n").await;
        let r = execute(parse_with(&format!(r#"{{ url: "{url}", within: 5s }}"#)))
            .await
            .unwrap();
        let last = r.outputs.get("last_event").unwrap();
        assert!(last["data"].is_null());
        assert_eq!(last["data_text"], serde_json::json!("not json"));
    }
}
