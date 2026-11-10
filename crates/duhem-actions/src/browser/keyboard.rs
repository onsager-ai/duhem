//! Keyboard and focus driver ops (#580): `press`, `focus`, `assertFocus`.

use serde_json::{Value, json};

use super::{Page, PwError};

impl Page {
    /// Press `key` (Playwright key vocabulary). With a `selector` the
    /// located element is focused and pressed (`locator.press`);
    /// without one the key goes to `page.keyboard`. Returns the
    /// receiving element's `{tag, id, role, name}` sampled before
    /// dispatch.
    pub async fn press(
        &self,
        selector: Option<&str>,
        key: &str,
        timeout_ms: f64,
    ) -> Result<Value, PwError> {
        let mut req = self.p();
        req["selector"] = json!(selector);
        req["key"] = json!(key);
        req["timeoutMs"] = json!(timeout_ms);
        self.conn.request("press", req).await
    }

    /// Programmatic `element.focus()` via `locator.focus`.
    pub async fn focus(&self, selector: &str, timeout_ms: f64) -> Result<(), PwError> {
        let mut req = self.p();
        req["selector"] = json!(selector);
        req["timeoutMs"] = json!(timeout_ms);
        self.conn.request("focus", req).await.map(|_| ())
    }

    /// Poll until the single element matching `selector` is focused
    /// (`focused = true`) or not focused (`focused = false`), or the
    /// deadline elapses. Returns `(satisfied, matches)`; `matches > 1`
    /// means the locator was ambiguous and nothing was judged.
    pub async fn assert_focus(
        &self,
        selector: &str,
        focused: bool,
        timeout_ms: f64,
    ) -> Result<(bool, u32), PwError> {
        let mut req = self.p();
        req["selector"] = json!(selector);
        req["focused"] = json!(focused);
        req["timeoutMs"] = json!(timeout_ms);
        let v = self.conn.request("assertFocus", req).await?;
        Ok((
            v["satisfied"].as_bool().unwrap_or(false),
            v["matches"].as_u64().unwrap_or(0) as u32,
        ))
    }
}
