//! `ui/assert-state` — observe a page-level state.
//!
//! Four states per the spec on issue #37:
//!
//! - `loaded` — `document.readyState === 'complete'` (the
//!   readyState value the browser sets on the `load` event).
//! - `network_idle` — no new resource entries observed for 500 ms.
//! - `authenticated` / `signed_out` — strictly observational checks
//!   for the presence (or absence) of an author-named cookie or
//!   local-storage key. The action carries no app-specific logic;
//!   the *meaning* of "authenticated" lives in the marker the
//!   Verification Definition chose.
//!
//! The state / marker pairing is enforced at deserialize time via
//! an internally-tagged enum: `marker:` is structurally required on
//! `authenticated` / `signed_out` and structurally rejected on
//! `loaded` / `network_idle`. No runtime branch validates this;
//! `serde` does.
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

/// Discriminated state. The internally-tagged `state:` field selects
/// the variant; `marker:` is structurally required on `authenticated`
/// / `signed_out` and structurally rejected on `loaded` /
/// `network_idle`. Each variant carries its own `within:` so the
/// outer `With` doesn't need to flatten.
#[derive(Debug, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum With {
    Loaded {
        #[serde(default)]
        within: Option<WithinSpec>,
    },
    NetworkIdle {
        #[serde(default)]
        within: Option<WithinSpec>,
    },
    Authenticated {
        marker: Marker,
        #[serde(default)]
        within: Option<WithinSpec>,
    },
    SignedOut {
        marker: Marker,
        #[serde(default)]
        within: Option<WithinSpec>,
    },
}

impl With {
    fn timeout(&self) -> Duration {
        let w = match self {
            With::Loaded { within }
            | With::NetworkIdle { within }
            | With::Authenticated { within, .. }
            | With::SignedOut { within, .. } => within,
        };
        w.map(Into::into).unwrap_or(DEFAULT_WITHIN)
    }
}

pub struct AssertState;

#[async_trait]
impl Action for AssertState {
    fn uses(&self) -> &'static str {
        "ui/assert-state"
    }

    fn contract(&self) -> crate::action::ActionContract {
        use crate::action::{ActionContract, FieldSpec};
        ActionContract {
            uses: "ui/assert-state",
            summary: "Assert an app state (e.g. signed in/out, or a marker) via `state:`.",
            with: vec![
                FieldSpec::required("state"),
                FieldSpec::optional("marker"),
                FieldSpec::optional("within"),
            ],
            outputs: vec!["satisfied"],
            example: "- uses: ui/assert-state\n  with: { state: signed_in }",
        }
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
        let timeout = with.timeout();
        let deadline = Instant::now() + timeout;

        let satisfied = loop {
            let observed = check_state(ctx, &with, deadline).await?;
            if observed {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            sleep(POLL_INTERVAL).await;
        };

        Ok(ActionResult::ok().with_output("satisfied", json!(satisfied)))
    }
}

async fn check_state(
    ctx: &ActionCtx<'_>,
    with: &With,
    deadline: Instant,
) -> Result<bool, ActionError> {
    match with {
        With::Loaded { .. } => check_loaded(ctx).await,
        With::NetworkIdle { .. } => check_network_idle(ctx, deadline).await,
        With::Authenticated { marker, .. } => marker_present(ctx, marker).await,
        With::SignedOut { marker, .. } => marker_present(ctx, marker).await.map(|p| !p),
    }
}

async fn check_loaded(ctx: &ActionCtx<'_>) -> Result<bool, ActionError> {
    let ready: String = ctx
        .require_page()?
        .eval("document.readyState")
        .await
        .map_err(|e| ActionError::Playwright(format!("ui/assert-state: readyState: {e}")))?;
    Ok(ready == "complete")
}

/// Heuristic network-idle: the count of `performance.resource`
/// entries must stay flat for `NETWORK_IDLE_QUIET`. Implemented
/// in-process (not via Playwright's `networkidle` load state)
/// because the Rust binding doesn't expose `wait_for_load_state`.
///
/// The probe bails as soon as `deadline` is reached so we don't
/// overshoot the user's `within:` budget — important when
/// `within:` is shorter than `NETWORK_IDLE_QUIET` (e.g. 200 ms).
async fn check_network_idle(ctx: &ActionCtx<'_>, deadline: Instant) -> Result<bool, ActionError> {
    let initial: u64 = ctx
        .require_page()?
        .eval("performance.getEntriesByType('resource').length")
        .await
        .map_err(|e| ActionError::Playwright(format!("ui/assert-state: resources: {e}")))?;

    let quiet_until = Instant::now() + NETWORK_IDLE_QUIET;
    loop {
        if Instant::now() >= quiet_until {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            // Out of `within:` budget. Let the outer loop record
            // `satisfied: false`; don't claim quietness we haven't
            // actually observed.
            return Ok(false);
        }
        sleep(POLL_INTERVAL).await;
        let now_count: u64 = ctx
            .require_page()?
            .eval("performance.getEntriesByType('resource').length")
            .await
            .map_err(|e| ActionError::Playwright(format!("ui/assert-state: resources: {e}")))?;
        if now_count != initial {
            return Ok(false);
        }
    }
}

async fn marker_present(ctx: &ActionCtx<'_>, marker: &Marker) -> Result<bool, ActionError> {
    match marker.kind {
        MarkerKind::Cookie => {
            let cookies =
                ctx.require_page()?.cookies().await.map_err(|e| {
                    ActionError::Playwright(format!("ui/assert-state: cookies: {e}"))
                })?;
            Ok(cookies.iter().any(|c| c.name == marker.name))
        }
        MarkerKind::LocalStorage => {
            let expr = format!("localStorage.getItem({}) !== null", json!(marker.name));
            let present: bool = ctx.require_page()?.eval(&expr).await.map_err(|e| {
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
        let w: With = serde_yml::from_str(yaml).unwrap();
        assert!(matches!(w, With::Loaded { .. }));
        assert_eq!(w.timeout(), Duration::from_secs(2));
    }

    #[test]
    fn parses_network_idle() {
        let yaml = r#"{ state: network_idle }"#;
        let w: With = serde_yml::from_str(yaml).unwrap();
        assert!(matches!(w, With::NetworkIdle { .. }));
    }

    #[test]
    fn parses_authenticated_with_cookie_marker() {
        let yaml = r#"
state: authenticated
marker: { kind: cookie, name: "session" }
within: 1s
"#;
        let w: With = serde_yml::from_str(yaml).unwrap();
        match w {
            With::Authenticated { ref marker, .. } => {
                assert_eq!(marker.kind, MarkerKind::Cookie);
                assert_eq!(marker.name, "session");
            }
            _ => panic!("expected Authenticated"),
        }
    }

    #[test]
    fn parses_signed_out_with_local_storage_marker() {
        let yaml = r#"
state: signed_out
marker: { kind: local_storage, name: "auth_token" }
"#;
        let w: With = serde_yml::from_str(yaml).unwrap();
        match w {
            With::SignedOut { ref marker, .. } => {
                assert_eq!(marker.kind, MarkerKind::LocalStorage);
            }
            _ => panic!("expected SignedOut"),
        }
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

    /// Spec promise: marker is required on the auth states. The
    /// internally-tagged enum's `marker:` field has no `Option`
    /// wrapper, so this fails at parse time.
    #[test]
    fn rejects_authenticated_without_marker() {
        let yaml = r#"{ state: authenticated }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    /// Spec promise: marker is rejected on the non-auth states.
    /// `Loaded` has no `marker:` field, so `deny_unknown_fields`
    /// rejects the mapping at parse time.
    #[test]
    fn rejects_loaded_with_marker() {
        let yaml = r#"
state: loaded
marker: { kind: cookie, name: "x" }
"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }
}
