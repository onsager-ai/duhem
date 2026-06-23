//! Browser driver — official Playwright via a Node sidecar (#71).
//!
//! Replaces the unmaintained `playwright = "0.0.20"` octaltree crate
//! (bundled Playwright 1.11.0, no Apple-Silicon support) with the
//! official, maintained Playwright Node package driven over a small
//! Duhem-owned stdio JSON-RPC protocol. The sidecar script lives at
//! `crates/duhem-actions/sidecar/index.mjs`; this module spawns it
//! (`node index.mjs`) and talks to it.
//!
//! ## Lifecycle
//!
//! - One sidecar process + one `Browser` per `duhem run` (held by
//!   [`RunBrowser`]).
//! - One `BrowserContext` + one `Page` per check (held by
//!   [`CheckBrowser`]). Cookies and storage are isolated per check —
//!   the "fresh user" intuition.
//! - Headless by default; `--headed` on `duhem run` flips
//!   [`RunBrowser::launch`]'s `headed` argument.
//!
//! ## Protocol & concurrency
//!
//! Newline-delimited JSON: `{id, op, ...}` requests, `{id, ok,
//! result|error}` responses. The runtime executes criteria → checks →
//! steps sequentially, so a single request/response channel guarded by
//! a mutex is sufficient — no request multiplexing.
//!
//! The Playwright *browser binary* is the operator's responsibility
//! (`npx playwright install chromium`), and Node ≥ 20 must be on PATH
//! (overridable via `DUHEM_NODE`). [`RunBrowser::launch`] fails fast
//! with a clear hint when either is missing.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::error::ActionError;

/// Error from the sidecar / driver. `Display` carries the underlying
/// Playwright message verbatim so the `ui/*` actions' existing
/// `is_timeout_message(&e.to_string())` classification keeps working.
#[derive(Debug)]
pub struct PwError(pub String);

impl std::fmt::Display for PwError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PwError {}

/// Element state for `waitForSelector`. Replaces the octaltree
/// `playwright::api::frame::FrameState`.
#[derive(Debug, Clone, Copy)]
pub enum ElementState {
    Attached,
    Detached,
    Visible,
    Hidden,
}

impl ElementState {
    fn as_str(self) -> &'static str {
        match self {
            ElementState::Attached => "attached",
            ElementState::Detached => "detached",
            ElementState::Visible => "visible",
            ElementState::Hidden => "hidden",
        }
    }
}

/// `selectOption` discriminator — exactly one of value / label / index.
#[derive(Debug)]
pub enum SelectBy {
    Value(String),
    Label(String),
    Index(usize),
}

impl SelectBy {
    fn to_json(&self) -> Value {
        match self {
            SelectBy::Value(v) => json!({ "value": v }),
            SelectBy::Label(l) => json!({ "label": l }),
            SelectBy::Index(i) => json!({ "index": i }),
        }
    }
}

/// A browser cookie. Extra Playwright fields are ignored on
/// deserialize; `ui/assert-state` only needs the name.
#[derive(Debug, Clone, Deserialize)]
pub struct Cookie {
    pub name: String,
}

/// One recorded HTTP response on a page, surfaced to `api/observe`
/// (#72). The sidecar's per-page recorder (`page.on('response', …)`)
/// materializes every field up front — including reading the body
/// eagerly — so the Rust side has no fallible accessors to drive: the
/// URL/method filter and body decode operate on plain data.
///
/// Bodies cross the wire base64-encoded (raw bytes survive JSON);
/// `api/observe` owns UTF-8-lossy rendering and JSON parsing, exactly
/// as the pre-#71 implementation did off the live Playwright objects.
/// `body_error` carries a body-read failure verbatim; `api/observe`
/// propagates it only for the matched event (collect-on-match), so an
/// unrelated failed response never breaks an unrelated observe.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkEvent {
    pub method: String,
    pub url: String,
    pub status: u16,
    #[serde(default)]
    pub request_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub request_body_base64: Option<String>,
    #[serde(default)]
    pub response_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body_base64: Option<String>,
    #[serde(default)]
    pub body_error: Option<String>,
}

/// A `pollNetwork` batch: recorded events from the requested cursor
/// onward, plus the new cursor (the recorder's buffer length) to pass
/// on the next poll.
#[derive(Debug, Clone, Deserialize)]
pub struct NetworkBatch {
    pub events: Vec<NetworkEvent>,
    pub cursor: u64,
}

#[derive(Deserialize)]
struct Response {
    id: u64,
    ok: bool,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

/// Shared sidecar connection. The mutex serializes request/response
/// turns; the runtime never issues concurrent browser ops.
struct Conn {
    inner: Mutex<ConnInner>,
}

struct ConnInner {
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Conn {
    async fn request(&self, op: &str, params: Value) -> Result<Value, PwError> {
        let mut guard = self.inner.lock().await;
        let id = guard.next_id;
        guard.next_id += 1;

        let mut obj = match params {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        obj.insert("id".into(), json!(id));
        obj.insert("op".into(), json!(op));
        let mut line = serde_json::to_string(&Value::Object(obj))
            .map_err(|e| PwError(format!("encode request: {e}")))?;
        line.push('\n');
        guard
            .stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| PwError(format!("sidecar write: {e}")))?;
        guard
            .stdin
            .flush()
            .await
            .map_err(|e| PwError(format!("sidecar flush: {e}")))?;

        loop {
            let line = match guard
                .stdout
                .next_line()
                .await
                .map_err(|e| PwError(format!("sidecar read: {e}")))?
            {
                Some(l) => l,
                None => return Err(PwError("sidecar closed the connection".into())),
            };
            let resp: Response = serde_json::from_str(&line)
                .map_err(|e| PwError(format!("decode sidecar response `{line}`: {e}")))?;
            if resp.id != id {
                continue; // not our response (defensive; channel is serial)
            }
            if resp.ok {
                return Ok(resp.result.unwrap_or(Value::Null));
            }
            return Err(PwError(
                resp.error.unwrap_or_else(|| "unknown sidecar error".into()),
            ));
        }
    }
}

fn node_command() -> String {
    std::env::var("DUHEM_NODE").unwrap_or_else(|_| "node".to_string())
}

fn sidecar_dir() -> PathBuf {
    match std::env::var("DUHEM_SIDECAR_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sidecar"),
    }
}

/// Fail fast if Node is absent or older than the supported floor
/// (Node 20 LTS; 18 is EOL). Returns the install hint as an
/// `ActionError` rather than letting a cryptic spawn failure surface.
async fn check_node_version(node: &str) -> Result<(), ActionError> {
    let out = Command::new(node)
        .arg("--version")
        .output()
        .await
        .map_err(|e| {
            ActionError::Playwright(format!(
                "Node.js not found (`{node} --version` failed: {e}). Install Node \u{2265} 20 (or set DUHEM_NODE)."
            ))
        })?;
    let ver = String::from_utf8_lossy(&out.stdout);
    let major = ver
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok());
    match major {
        Some(m) if m >= 20 => Ok(()),
        Some(m) => Err(ActionError::Playwright(format!(
            "Node.js {m} is too old; the Playwright sidecar requires Node \u{2265} 20. Upgrade Node (or set DUHEM_NODE)."
        ))),
        None => Err(ActionError::Playwright(format!(
            "could not parse Node version from `{}`",
            ver.trim()
        ))),
    }
}

/// Recognize the Playwright "browser binary missing" / sidecar-deps
/// failure modes and emit an actionable hint. Other errors pass
/// through verbatim.
pub(crate) fn humanize_launch_error(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("executable doesn't exist")
        || lower.contains("install missing dependencies")
        || lower.contains("browsertype.launch")
        || lower.contains("looks like playwright")
    {
        format!(
            "chromium binary not installed, and no existing browser was found to fall back to. Run `npx playwright install chromium` once, or set `DUHEM_BROWSER_EXECUTABLE=/path/to/chrome` to use a browser already on this machine, and retry. (driver said: {raw})"
        )
    } else if lower.contains("cannot find package 'playwright'")
        || lower.contains("err_module_not_found")
    {
        format!(
            "the Playwright sidecar's dependencies are not installed. Run `npm ci` in crates/duhem-actions/sidecar and retry. (node said: {raw})"
        )
    } else {
        raw.to_string()
    }
}

/// Sidecar process + browser shared for the lifetime of a `duhem run`.
/// Drop kills the sidecar.
pub struct RunBrowser {
    child: Child,
    conn: Arc<Conn>,
}

impl RunBrowser {
    /// Spawn the sidecar and launch chromium. `headed = false` (the
    /// default) runs headless. Fails fast on missing Node, missing
    /// sidecar, or missing browser binary, each with a clear hint.
    pub async fn launch(headed: bool) -> Result<Self, ActionError> {
        let node = node_command();
        check_node_version(&node).await?;

        let dir = sidecar_dir();
        let index = dir.join("index.mjs");
        if !index.exists() {
            return Err(ActionError::Playwright(format!(
                "Playwright sidecar not found at {} (set DUHEM_SIDECAR_DIR to override)",
                index.display()
            )));
        }

        let mut child = Command::new(&node)
            .arg(&index)
            .current_dir(&dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                ActionError::Playwright(format!("failed to spawn node sidecar (`{node}`): {e}"))
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ActionError::Playwright("sidecar stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ActionError::Playwright("sidecar stdout unavailable".into()))?;
        let conn = Arc::new(Conn {
            inner: Mutex::new(ConnInner {
                stdin,
                stdout: BufReader::new(stdout).lines(),
                next_id: 1,
            }),
        });

        conn.request("launch", json!({ "headless": !headed }))
            .await
            .map_err(|e| ActionError::Playwright(humanize_launch_error(&e.to_string())))?;

        Ok(Self { child, conn })
    }

    /// Allocate a fresh context + page for one check.
    pub async fn open_check(&self) -> Result<CheckBrowser, ActionError> {
        let ctx = self
            .conn
            .request("newContext", json!({}))
            .await
            .map_err(|e| ActionError::Playwright(format!("context: {e}")))?;
        let context_id = ctx
            .get("contextId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ActionError::Playwright("newContext: missing contextId".into()))?
            .to_string();

        let pg = self
            .conn
            .request("newPage", json!({ "contextId": context_id }))
            .await
            .map_err(|e| ActionError::Playwright(format!("page: {e}")))?;
        let page_id = pg
            .get("pageId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ActionError::Playwright("newPage: missing pageId".into()))?
            .to_string();

        Ok(CheckBrowser {
            conn: self.conn.clone(),
            context_id,
            page: Page {
                conn: self.conn.clone(),
                id: page_id,
            },
        })
    }
}

impl Drop for RunBrowser {
    fn drop(&mut self) {
        // Best-effort: closing stdin would also make the sidecar exit,
        // but an explicit kill is the reliable teardown from a sync Drop.
        let _ = self.child.start_kill();
    }
}

/// Per-check handle. `context_id` is held alongside `page` so
/// [`CheckBrowser::close`] can tear the context down (which disposes
/// the page).
pub struct CheckBrowser {
    conn: Arc<Conn>,
    context_id: String,
    pub page: Page,
}

impl CheckBrowser {
    /// Explicitly close the context. Drop of [`RunBrowser`] also tears
    /// the sidecar down; this is for callers that want close-failure
    /// surfaced per check.
    pub async fn close(self) -> Result<(), ActionError> {
        self.conn
            .request("closeContext", json!({ "contextId": self.context_id }))
            .await
            .map(|_| ())
            .map_err(|e| ActionError::Playwright(format!("close: {e}")))
    }
}

/// Per-check browser page. Methods mirror the subset of the Playwright
/// `Page` API the `ui/*` actions use; each forwards one sidecar op.
/// Errors carry the driver message verbatim (see [`PwError`]).
pub struct Page {
    conn: Arc<Conn>,
    id: String,
}

impl Page {
    fn p(&self) -> Value {
        json!({ "pageId": self.id })
    }

    pub async fn goto(&self, url: &str, timeout_ms: f64) -> Result<(), PwError> {
        let mut req = self.p();
        req["url"] = json!(url);
        req["timeoutMs"] = json!(timeout_ms);
        self.conn.request("goto", req).await.map(|_| ())
    }

    pub async fn click(&self, selector: &str, timeout_ms: f64) -> Result<(), PwError> {
        let mut req = self.p();
        req["selector"] = json!(selector);
        req["timeoutMs"] = json!(timeout_ms);
        self.conn.request("click", req).await.map(|_| ())
    }

    pub async fn fill(&self, selector: &str, text: &str, timeout_ms: f64) -> Result<(), PwError> {
        let mut req = self.p();
        req["selector"] = json!(selector);
        req["text"] = json!(text);
        req["timeoutMs"] = json!(timeout_ms);
        self.conn.request("fill", req).await.map(|_| ())
    }

    /// Append text (no clear) — the `ui/type clear:false` path.
    pub async fn type_text(
        &self,
        selector: &str,
        text: &str,
        timeout_ms: f64,
    ) -> Result<(), PwError> {
        let mut req = self.p();
        req["selector"] = json!(selector);
        req["text"] = json!(text);
        req["timeoutMs"] = json!(timeout_ms);
        self.conn.request("type", req).await.map(|_| ())
    }

    pub async fn select_option(
        &self,
        selector: &str,
        by: &SelectBy,
        timeout_ms: f64,
    ) -> Result<(), PwError> {
        let mut req = self.p();
        req["selector"] = json!(selector);
        req["by"] = by.to_json();
        req["timeoutMs"] = json!(timeout_ms);
        self.conn.request("selectOption", req).await.map(|_| ())
    }

    /// Wait for `selector` to reach `state`. `Ok(())` on success;
    /// `Err` whose message contains "Timeout" when the deadline
    /// elapses (the actions map that to `satisfied: false`).
    pub async fn wait_for_selector(
        &self,
        selector: &str,
        state: ElementState,
        timeout_ms: f64,
    ) -> Result<(), PwError> {
        let mut req = self.p();
        req["selector"] = json!(selector);
        req["state"] = json!(state.as_str());
        req["timeoutMs"] = json!(timeout_ms);
        self.conn.request("waitForSelector", req).await.map(|_| ())
    }

    /// Number of elements matching `selector` at observation time.
    pub async fn count(&self, selector: &str) -> Result<u32, PwError> {
        let mut req = self.p();
        req["selector"] = json!(selector);
        let v = self.conn.request("count", req).await?;
        Ok(v.as_u64().unwrap_or(0) as u32)
    }

    pub async fn url(&self) -> Result<String, PwError> {
        let v = self.conn.request("url", self.p()).await?;
        Ok(v.as_str().unwrap_or("").to_string())
    }

    /// Evaluate a JS expression in the page and deserialize the result.
    pub async fn eval<T: serde::de::DeserializeOwned>(&self, expr: &str) -> Result<T, PwError> {
        let mut req = self.p();
        req["expr"] = json!(expr);
        let v = self.conn.request("eval", req).await?;
        serde_json::from_value(v).map_err(|e| PwError(format!("eval result decode: {e}")))
    }

    pub async fn cookies(&self) -> Result<Vec<Cookie>, PwError> {
        let v = self.conn.request("cookies", self.p()).await?;
        serde_json::from_value(v).map_err(|e| PwError(format!("cookies decode: {e}")))
    }

    /// Drain recorded network responses from `cursor` onward. Returns
    /// the batch plus the next cursor; `api/observe` polls this within
    /// its `within:` window. See [`NetworkEvent`].
    pub async fn poll_network(&self, cursor: u64) -> Result<NetworkBatch, PwError> {
        let mut req = self.p();
        req["cursor"] = json!(cursor);
        let v = self.conn.request("pollNetwork", req).await?;
        serde_json::from_value(v).map_err(|e| PwError(format!("pollNetwork decode: {e}")))
    }
}

#[cfg(test)]
mod humanize_tests {
    use super::humanize_launch_error;

    #[test]
    fn missing_browser_names_both_remediations() {
        // The sidecar's discovered-browser fallback (#105) runs first;
        // this message is the floor when no browser exists anywhere, so
        // it must point at both `playwright install` and the
        // DUHEM_BROWSER_EXECUTABLE override.
        let msg = humanize_launch_error(
            "browserType.launch: Executable doesn't exist at /…/chrome-headless-shell",
        );
        assert!(msg.contains("DUHEM_BROWSER_EXECUTABLE"), "got: {msg}");
        assert!(msg.contains("playwright install chromium"), "got: {msg}");
    }

    #[test]
    fn missing_sidecar_deps_message_unchanged() {
        let msg = humanize_launch_error("Cannot find package 'playwright' imported from …");
        assert!(msg.contains("npm ci"), "got: {msg}");
    }

    #[test]
    fn unrelated_error_passes_through() {
        let msg = humanize_launch_error("some other failure");
        assert_eq!(msg, "some other failure");
    }
}
