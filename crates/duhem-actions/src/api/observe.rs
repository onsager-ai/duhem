//! `api/observe` — passive HTTP observation via the browser's network
//! events.
//!
//! ## Temporarily stubbed (#71 → #72)
//!
//! `api/observe` taps Playwright's live network-event *stream*
//! (`Response` / `Request` objects, their bodies and headers). When the
//! browser driver moved to the official-Playwright Node sidecar (#71),
//! the simple request/response RPC the `ui/*` ops use was not enough
//! for streaming network events — and because every action shares one
//! `ActionCtx.page` type, `api/observe` could not stay on the old
//! octaltree crate while `ui/*` migrated. So the network-observation
//! channel is deferred to **onsager-ai/duhem#72**, and `invoke` here
//! returns a clear `ActionError` until it lands.
//!
//! The `With` schema is retained (and still validated) so Verification
//! Definitions referencing `api/observe` parse and `duhem validate`
//! cleanly — they fail at run time with the message below, not at parse
//! time. The matcher / body-decode logic and its tests return with the
//! real implementation in #72.

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult};
use crate::error::ActionError;
use crate::with::WithinSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct With {
    /// Optional method filter. Omitted = match any method.
    #[serde(default)]
    #[allow(dead_code)]
    method: Option<String>,
    /// URL pattern. Exact string match by default; regex when prefixed
    /// `re:`.
    #[allow(dead_code)]
    url_pattern: String,
    /// Reserved for the future concurrent-listener engine extension.
    #[serde(default)]
    #[allow(dead_code)]
    after: Option<String>,
    /// Max wait for a matching event.
    #[serde(default)]
    #[allow(dead_code)]
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
        _ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        // Validate the shape so authors get a parse-time error for a
        // malformed `with:` rather than the deferral message below.
        let _with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "api/observe",
                source: e,
            })?;

        Err(ActionError::Playwright(
            "api/observe is temporarily unsupported on the Playwright sidecar driver; \
             its network-observation channel is tracked in onsager-ai/duhem#72"
                .to_string(),
        ))
    }
}

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
        let r: Result<With, _> = serde_yml::from_str(r#"{ method: GET }"#);
        assert!(r.is_err());
    }
}
