//! `ui/assert-state` — observe a page-level state.
//!
//! Four states per the spec on issue #37:
//!
//! - `loaded` — `document.readyState === 'complete'`.
//! - `network_idle` — no new resource entries observed for 500 ms.
//! - `authenticated` / `signed_out` — strictly observational checks
//!   for the presence (or absence) of an author-named cookie or
//!   local-storage key. The action carries no app-specific logic;
//!   the *meaning* of "authenticated" lives in the marker the
//!   Verification Definition chose.
//!
//! Outputs in every case: `satisfied: bool`. Timeout shape matches
//! `ui/assert-element` — the wait-with-deadline returns
//! `Outcome::Ok` with `satisfied: false`, not `Outcome::Timeout`.
//! `Outcome::Timeout` is reserved for browser-level failures
//! (driver hang) per the §11.1 structural choice.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use tokio::time::sleep;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::error::ActionError;
use crate::with::WithinSpec;

/// How long the resource-entry count must stay flat for
/// `network_idle` to fire. Matches Playwright's documented
/// `networkidle` semantics ("no network connections for at
/// least 500 ms").
const NETWORK_IDLE_QUIET: Duration = Duration::from_millis(500);

/// Polling interval for every state. Small enough that 200 ms
/// `within:` produces multiple samples.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageState {
    Loaded,
    NetworkIdle,
    Authenticated,
    SignedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MarkerKind {
    Cookie,
    LocalStorage,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    kind: MarkerKind,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    state: PageState,
    #[serde(default)]
    marker: Option<Marker>,
    #[serde(default)]
    within: Option<WithinSpec>,
}

pub struct AssertState;

#[async_trait]
impl Action for AssertState {
    fn uses(&self) -> &'static str {
        "ui/assert-state"
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "ui/assert-state",
                source: e,
            })?;
        let timeout: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);

        // Marker is required for the auth states and forbidden for
        // the others — the validator runs at invocation time so
        // authors get the schema error before the deadline ticks.
        let needs_marker = matches!(with.state, PageState::Authenticated | PageState::SignedOut);
        match (needs_marker, &with.marker) {
            (true, None) => {
                return Err(ActionError::InvalidWith {
                    action: "ui/assert-state",
                    source: serde::de::Error::custom(
                        "`marker:` is required for state `authenticated` / `signed_out`",
                    ),
                });
            }
            (false, Some(_)) => {
                return Err(ActionError::InvalidWith {
                    action: "ui/assert-state",
                    source: serde::de::Error::custom(
                        "`marker:` is only valid for state `authenticated` / `signed_out`",
                    ),
                });
            }
            _ => {}
        }

        let started = Instant::now();
        let satisfied = wait_until(timeout, started, || async {
            check_state(ctx, with.state, with.marker.as_ref()).await
        })
        .await?;

        Ok(ActionResult::ok().with_output("satisfied", json!(satisfied)))
    }
}

/// Poll `probe` every `POLL_INTERVAL` until it returns `true` or
/// `timeout` elapses. Probe errors propagate; a poll that runs out
/// of time returns `Ok(false)` per the assert-element convention.
async fn wait_until<F, Fut>(
    timeout: Duration,
    started: Instant,
    mut probe: F,
) -> Result<bool, ActionError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool, ActionError>>,
{
    loop {
        if probe().await? {
            return Ok(true);
        }
        if started.elapsed() >= timeout {
            return Ok(false);
        }
        sleep(POLL_INTERVAL).await;
    }
}

async fn check_state(
    ctx: &ActionCtx<'_>,
    state: PageState,
    marker: Option<&Marker>,
) -> Result<bool, ActionError> {
    match state {
        PageState::Loaded => check_loaded(ctx).await,
        PageState::NetworkIdle => check_network_idle(ctx).await,
        PageState::Authenticated => marker_present(ctx, marker.expect("validated above")).await,
        PageState::SignedOut => marker_present(ctx, marker.expect("validated above"))
            .await
            .map(|present| !present),
    }
}

async fn check_loaded(ctx: &ActionCtx<'_>) -> Result<bool, ActionError> {
    let ready: String = ctx
        .page
        .eval("document.readyState")
        .await
        .map_err(|e| ActionError::Playwright(format!("ui/assert-state: readyState: {e}")))?;
    Ok(ready == "complete")
}

/// Heuristic network-idle: the count of `performance.resource`
/// entries must stay flat for `NETWORK_IDLE_QUIET`. Implemented
/// in-process (not via Playwright's `networkidle` load state)
/// because the Rust binding doesn't expose `wait_for_load_state`.
async fn check_network_idle(ctx: &ActionCtx<'_>) -> Result<bool, ActionError> {
    let initial: u64 = ctx
        .page
        .eval("performance.getEntriesByType('resource').length")
        .await
        .map_err(|e| ActionError::Playwright(format!("ui/assert-state: resources: {e}")))?;

    // Hold the count steady across the quiet window. If it moves,
    // bail; the outer wait loop will resample.
    let deadline = Instant::now() + NETWORK_IDLE_QUIET;
    while Instant::now() < deadline {
        sleep(POLL_INTERVAL).await;
        let now_count: u64 = ctx
            .page
            .eval("performance.getEntriesByType('resource').length")
            .await
            .map_err(|e| ActionError::Playwright(format!("ui/assert-state: resources: {e}")))?;
        if now_count != initial {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn marker_present(ctx: &ActionCtx<'_>, marker: &Marker) -> Result<bool, ActionError> {
    match marker.kind {
        MarkerKind::Cookie => {
            let cookies =
                ctx.page.context().cookies(&[]).await.map_err(|e| {
                    ActionError::Playwright(format!("ui/assert-state: cookies: {e}"))
                })?;
            Ok(cookies.iter().any(|c| c.name == marker.name))
        }
        MarkerKind::LocalStorage => {
            let expr = format!("localStorage.getItem({}) !== null", json!(marker.name));
            let present: bool = ctx.page.eval(&expr).await.map_err(|e| {
                ActionError::Playwright(format!("ui/assert-state: localStorage: {e}"))
            })?;
            Ok(present)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_loaded() {
        let yaml = r#"{ state: loaded, within: 2s }"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert_eq!(v.state, PageState::Loaded);
        assert!(v.marker.is_none());
    }

    #[test]
    fn parses_network_idle() {
        let yaml = r#"{ state: network_idle }"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert_eq!(v.state, PageState::NetworkIdle);
    }

    #[test]
    fn parses_authenticated_with_cookie_marker() {
        let yaml = r#"
state: authenticated
marker: { kind: cookie, name: "session" }
within: 1s
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert_eq!(v.state, PageState::Authenticated);
        let m = v.marker.as_ref().unwrap();
        assert_eq!(m.kind, MarkerKind::Cookie);
        assert_eq!(m.name, "session");
    }

    #[test]
    fn parses_signed_out_with_local_storage_marker() {
        let yaml = r#"
state: signed_out
marker: { kind: local_storage, name: "auth_token" }
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert_eq!(v.state, PageState::SignedOut);
        assert_eq!(v.marker.as_ref().unwrap().kind, MarkerKind::LocalStorage);
    }

    #[test]
    fn rejects_unknown_state() {
        let yaml = r#"{ state: kind_of_loaded }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = r#"{ state: loaded, extra: 1 }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn rejects_unknown_marker_kind() {
        let yaml = r#"
state: authenticated
marker: { kind: jwt, name: "x" }
"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }
}
