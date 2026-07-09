//! HAR 1.2 serialization for network-tail capture (spec #204).
//!
//! Turns the browser page's recorded [`NetworkEvent`] buffer into a
//! valid HAR 1.2 log so a failing `ui/*` check's traffic opens in any
//! browser devtools / HAR viewer. The recorder (`api/observe`'s
//! source) captures method / url / status / headers / bodies but no
//! per-request timing or on-wire byte counts, so the fields HAR
//! requires and we don't record (`startedDateTime`, `timings`,
//! `headersSize`, `bodySize`) are emitted as honest stubs (epoch /
//! `-1`) rather than fabricated. Response `content.size` is the real
//! decoded body length.
//!
//! Redaction is not optional here: network events reliably carry
//! secrets (`Authorization` / `Cookie` headers, credentials in auth
//! request bodies) and capture blobs ship to the hub. Sensitive
//! headers are always redacted; a request that carried one has its
//! body redacted too (the auth-flow heuristic); bodies over
//! [`DEFAULT_BODY_CAP`] are truncated and marked. The serializer is
//! pure — no clock, no I/O — so it is deterministic and unit-testable.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use duhem_actions::NetworkEvent;
use serde_json::{Value, json};

/// Reserved output name for the network capture (under the #202
/// `capture/` namespace).
pub(crate) const CAPTURE_NETWORK: &str = "capture/network";

/// Keep only the last N recorded events — the tail at failure time,
/// not the whole run.
pub(crate) const NETWORK_TAIL: usize = 50;

/// Per-body ceiling before truncation (32 KiB). Bounds blob size on a
/// page that streamed a large response.
pub(crate) const DEFAULT_BODY_CAP: usize = 32 * 1024;

/// Header names whose values are always redacted (compared
/// lowercase). A request carrying any of these also has its body
/// redacted.
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "proxy-authorization",
];

const REDACTED: &str = "<redacted>";

fn is_sensitive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE_HEADERS.contains(&lower.as_str())
}

/// Whether the request carried any sensitive header — the signal that
/// its body is likely credential-bearing (a login POST) and must be
/// redacted.
fn request_is_auth(evt: &NetworkEvent) -> bool {
    evt.request_headers.keys().any(|k| is_sensitive(k))
}

/// HAR `headers` array with sensitive values redacted.
fn har_headers(headers: &std::collections::BTreeMap<String, String>) -> Value {
    let arr: Vec<Value> = headers
        .iter()
        .map(|(name, value)| {
            let value = if is_sensitive(name) { REDACTED } else { value };
            json!({ "name": name, "value": value })
        })
        .collect();
    Value::Array(arr)
}

/// Decode a base64 body to a HAR text field, bounded by `cap`.
/// Returns `(text, size, encoding, truncated)`. UTF-8 decodes to
/// plain text; binary stays base64 (HAR's `encoding: "base64"`).
fn decode_body(b64: &str, cap: usize) -> (String, usize, Option<&'static str>, bool) {
    let bytes = BASE64.decode(b64).unwrap_or_default();
    let size = bytes.len();
    let (bytes, truncated) = if bytes.len() > cap {
        (&bytes[..cap], true)
    } else {
        (&bytes[..], false)
    };
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), size, None, truncated),
        Err(_) => (BASE64.encode(bytes), size, Some("base64"), truncated),
    }
}

/// Compose the body-marker comment from the decode flags. Both signals
/// coexist — a binary body that is also truncated keeps its `base64`
/// marker (they used to clobber each other in `postData`).
fn body_marker(encoding: Option<&str>, truncated: bool) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(enc) = encoding {
        parts.push(format!("encoding: {enc}"));
    }
    if truncated {
        parts.push("truncated".to_string());
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn mime_of(headers: &std::collections::BTreeMap<String, String>) -> String {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

fn request_json(evt: &NetworkEvent, cap: usize) -> Value {
    let mut req = json!({
        "method": evt.method,
        "url": evt.url,
        "httpVersion": "HTTP/1.1",
        "headers": har_headers(&evt.request_headers),
        "queryString": [],
        "cookies": [],
        "headersSize": -1,
        "bodySize": evt.request_body_base64.as_ref().map_or(0, |_| -1),
    });
    if let Some(b64) = &evt.request_body_base64 {
        let post = if request_is_auth(evt) {
            // Auth-flow heuristic: a request with a sensitive header
            // likely carries credentials in its body — redact it.
            json!({ "mimeType": mime_of(&evt.request_headers), "text": REDACTED })
        } else {
            let (text, _size, encoding, truncated) = decode_body(b64, cap);
            let mut pd = json!({ "mimeType": mime_of(&evt.request_headers), "text": text });
            if let Some(marker) = body_marker(encoding, truncated) {
                pd["comment"] = json!(marker);
            }
            pd
        };
        req["postData"] = post;
    }
    req
}

fn response_json(evt: &NetworkEvent, cap: usize) -> Value {
    let mut content = json!({
        "size": 0,
        "mimeType": mime_of(&evt.response_headers),
    });
    if let Some(b64) = &evt.body_base64 {
        let (text, size, encoding, truncated) = decode_body(b64, cap);
        content["size"] = json!(size);
        content["text"] = json!(text);
        if let Some(enc) = encoding {
            content["encoding"] = json!(enc);
        }
        if truncated {
            content["comment"] = json!("truncated");
        }
    } else if let Some(err) = &evt.body_error {
        content["comment"] = json!(format!("body unavailable: {err}"));
    }
    json!({
        "status": evt.status,
        "statusText": "",
        "httpVersion": "HTTP/1.1",
        "headers": har_headers(&evt.response_headers),
        "cookies": [],
        "content": content,
        "redirectURL": "",
        "headersSize": -1,
        "bodySize": -1,
    })
}

fn entry_json(evt: &NetworkEvent, cap: usize) -> Value {
    json!({
        // We record no per-request start time; a fixed epoch stub keeps
        // the HAR valid without fabricating timing.
        "startedDateTime": "1970-01-01T00:00:00.000Z",
        "time": -1,
        "request": request_json(evt, cap),
        "response": response_json(evt, cap),
        "cache": {},
        "timings": { "send": -1, "wait": -1, "receive": -1 },
    })
}

/// Serialize the tail of a page's recorded traffic to a HAR 1.2 log
/// string. Redacts sensitive headers/bodies and caps body size. Pure.
pub(crate) fn to_har(events: &[NetworkEvent], body_cap: usize) -> String {
    let tail = if events.len() > NETWORK_TAIL {
        &events[events.len() - NETWORK_TAIL..]
    } else {
        events
    };
    let entries: Vec<Value> = tail.iter().map(|e| entry_json(e, body_cap)).collect();
    let log = json!({
        "log": {
            "version": "1.2",
            "creator": { "name": "duhem", "version": env!("CARGO_PKG_VERSION") },
            "entries": entries,
        }
    });
    // Compact is fine — this is machine-read evidence, not a diff.
    log.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn evt() -> NetworkEvent {
        NetworkEvent {
            method: "GET".into(),
            url: "http://x/".into(),
            status: 200,
            request_headers: BTreeMap::new(),
            request_body_base64: None,
            response_headers: BTreeMap::new(),
            body_base64: None,
            body_error: None,
        }
    }

    fn b64(s: &str) -> String {
        BASE64.encode(s.as_bytes())
    }

    #[test]
    fn produces_valid_har_1_2_envelope() {
        let har = to_har(&[evt()], DEFAULT_BODY_CAP);
        let v: Value = serde_json::from_str(&har).unwrap();
        assert_eq!(v["log"]["version"], "1.2");
        assert_eq!(v["log"]["creator"]["name"], "duhem");
        assert_eq!(v["log"]["entries"].as_array().unwrap().len(), 1);
        assert_eq!(v["log"]["entries"][0]["response"]["status"], 200);
    }

    #[test]
    fn sensitive_headers_are_redacted_both_directions() {
        let mut e = evt();
        e.request_headers
            .insert("Authorization".into(), "Bearer sk-secret".into());
        e.response_headers
            .insert("Set-Cookie".into(), "session=abc".into());
        e.response_headers
            .insert("Content-Type".into(), "text/plain".into());
        let v: Value = serde_json::from_str(&to_har(&[e], DEFAULT_BODY_CAP)).unwrap();
        let req_h = &v["log"]["entries"][0]["request"]["headers"];
        assert_eq!(req_h[0]["name"], "Authorization");
        assert_eq!(req_h[0]["value"], REDACTED);
        let resp_h = v["log"]["entries"][0]["response"]["headers"]
            .as_array()
            .unwrap();
        let cookie = resp_h.iter().find(|h| h["name"] == "Set-Cookie").unwrap();
        assert_eq!(cookie["value"], REDACTED);
        let ct = resp_h.iter().find(|h| h["name"] == "Content-Type").unwrap();
        assert_eq!(ct["value"], "text/plain", "non-sensitive header verbatim");
    }

    #[test]
    fn auth_request_body_is_redacted() {
        let mut e = evt();
        e.method = "POST".into();
        e.request_headers
            .insert("cookie".into(), "session=abc".into());
        e.request_body_base64 = Some(b64(r#"{"password":"hunter2"}"#));
        let v: Value = serde_json::from_str(&to_har(&[e], DEFAULT_BODY_CAP)).unwrap();
        assert_eq!(
            v["log"]["entries"][0]["request"]["postData"]["text"],
            REDACTED
        );
    }

    #[test]
    fn non_auth_request_body_is_kept() {
        let mut e = evt();
        e.method = "POST".into();
        e.request_headers
            .insert("content-type".into(), "application/json".into());
        e.request_body_base64 = Some(b64(r#"{"q":"ok"}"#));
        let v: Value = serde_json::from_str(&to_har(&[e], DEFAULT_BODY_CAP)).unwrap();
        assert_eq!(
            v["log"]["entries"][0]["request"]["postData"]["text"],
            r#"{"q":"ok"}"#
        );
    }

    #[test]
    fn response_body_over_cap_is_truncated_and_marked() {
        let mut e = evt();
        let big = "a".repeat(100);
        e.body_base64 = Some(b64(&big));
        let v: Value = serde_json::from_str(&to_har(&[e], 10)).unwrap();
        let content = &v["log"]["entries"][0]["response"]["content"];
        assert_eq!(content["size"], 100, "size reports the true length");
        assert_eq!(content["text"].as_str().unwrap().len(), 10);
        assert_eq!(content["comment"], "truncated");
    }

    #[test]
    fn tail_keeps_only_the_last_n() {
        let events: Vec<NetworkEvent> = (0..NETWORK_TAIL + 5)
            .map(|i| {
                let mut e = evt();
                e.url = format!("http://x/{i}");
                e
            })
            .collect();
        let v: Value = serde_json::from_str(&to_har(&events, DEFAULT_BODY_CAP)).unwrap();
        let entries = v["log"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), NETWORK_TAIL);
        // First kept entry is event #5 (the oldest 5 dropped).
        assert_eq!(entries[0]["request"]["url"], "http://x/5");
    }

    #[test]
    fn binary_response_body_stays_base64() {
        let mut e = evt();
        // 0xFF 0xFE is invalid UTF-8.
        e.body_base64 = Some(BASE64.encode([0xFF, 0xFE, 0x00]));
        let v: Value = serde_json::from_str(&to_har(&[e], DEFAULT_BODY_CAP)).unwrap();
        assert_eq!(
            v["log"]["entries"][0]["response"]["content"]["encoding"],
            "base64"
        );
    }

    #[test]
    fn truncated_binary_request_body_keeps_both_markers() {
        // A non-auth (so kept) binary request body that also overflows
        // the cap must not lose its base64 marker to the truncation
        // marker — both coexist in the postData comment.
        let mut e = evt();
        e.method = "POST".into();
        e.request_headers
            .insert("content-type".into(), "application/octet-stream".into());
        e.request_body_base64 = Some(BASE64.encode([0xFFu8; 100]));
        let v: Value = serde_json::from_str(&to_har(&[e], 10)).unwrap();
        let comment = v["log"]["entries"][0]["request"]["postData"]["comment"]
            .as_str()
            .unwrap();
        assert!(comment.contains("base64"), "lost base64 marker: {comment}");
        assert!(
            comment.contains("truncated"),
            "lost truncated marker: {comment}"
        );
    }
}
