//! `ui/assert-url` — observe the current page URL.
//!
//! Like `ui/assert-element`, this is a *waiter*: it polls the URL
//! until the matcher succeeds or the deadline elapses. Matcher
//! success within the deadline → `satisfied: true`. Deadline
//! elapses with the URL never matching → `Outcome::Timeout`
//! per the spec on issue #37 (the URL never reaching the
//! expectation is "we didn't get to where we said we would",
//! which is the timeout-shaped outcome the judge maps to
//! `Inconclusive(Timeout)`). The two sibling waiters
//! (`ui/assert-element`, `ui/assert-state`) instead return
//! `Outcome::Ok` with `satisfied: false`; the divergence is by
//! design and is also called out in the `ui::` module doc.
//!
//! Exactly one of `equals:` (literal string match) or `matches:`
//! (regex) is required. The constraint is enforced at deserialize
//! time via an untagged enum — a mapping with both fields, or
//! with neither, fails to match any variant and is rejected
//! before `invoke`. The regex itself is compiled at the same
//! time, so syntactically bad patterns also surface at parse.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use tokio::time::sleep;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::error::ActionError;
use crate::with::WithinSpec;

/// Inter-poll sleep while waiting for the URL to match. Small
/// enough that a 200ms `within:` produces multiple samples;
/// large enough that we don't burn CPU against a stable URL.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// `With` is an untagged enum so the `equals` / `matches` exclusivity
/// is enforced by serde rather than at invocation time. Each variant
/// carries `deny_unknown_fields` so a mapping that sets both fields
/// fails to match any variant.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum With {
    Equals(EqualsArgs),
    Matches(MatchesArgs),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EqualsArgs {
    equals: String,
    #[serde(default)]
    within: Option<WithinSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MatchesArgs {
    /// Stored as the raw string; compiled into a `Regex` via
    /// `try_into` below so invalid patterns surface at parse time.
    matches: String,
    #[serde(default)]
    within: Option<WithinSpec>,
}

#[derive(Debug)]
enum Matcher {
    Equals(String),
    Matches(Regex),
}

#[derive(Debug)]
struct Plan {
    matcher: Matcher,
    timeout: Duration,
}

impl Plan {
    fn from_with(w: With) -> Result<Self, ActionError> {
        match w {
            With::Equals(a) => Ok(Plan {
                matcher: Matcher::Equals(a.equals),
                timeout: a.within.map(Into::into).unwrap_or(DEFAULT_WITHIN),
            }),
            With::Matches(a) => {
                let re = Regex::new(&a.matches).map_err(|e| ActionError::InvalidWith {
                    action: "ui/assert-url",
                    source: serde::de::Error::custom(format!("invalid regex: {e}")),
                })?;
                Ok(Plan {
                    matcher: Matcher::Matches(re),
                    timeout: a.within.map(Into::into).unwrap_or(DEFAULT_WITHIN),
                })
            }
        }
    }
}

impl Matcher {
    fn check(&self, url: &str) -> bool {
        match self {
            Matcher::Equals(s) => url == s,
            Matcher::Matches(re) => re.is_match(url),
        }
    }
}

pub struct AssertUrl;

#[async_trait]
impl Action for AssertUrl {
    fn uses(&self) -> &'static str {
        "ui/assert-url"
    }

    fn contract(&self) -> crate::action::ActionContract {
        use crate::action::{ActionContract, FieldSpec};
        ActionContract {
            uses: "ui/assert-url",
            summary: "Assert the page URL equals a string or matches a regex (exactly one of equals/matches).",
            with: vec![
                FieldSpec::optional("equals"),
                FieldSpec::optional("matches"),
                FieldSpec::optional("within"),
            ],
            outputs: vec!["satisfied", "actual"],
            example: "- uses: ui/assert-url\n  with: { equals: https://example.com/ }\n  outputs: { satisfied: satisfied }",
        }
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let page = ctx.require_page()?;
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "ui/assert-url",
                source: e,
            })?;
        let plan = Plan::from_with(with)?;

        let started = Instant::now();
        loop {
            let last_url = page
                .url()
                .await
                .map_err(|e| ActionError::Playwright(format!("ui/assert-url: url: {e}")))?;
            if plan.matcher.check(&last_url) {
                return Ok(ActionResult::ok()
                    .with_output("satisfied", json!(true))
                    .with_output("actual", json!(last_url)));
            }
            if started.elapsed() >= plan.timeout {
                return Ok(ActionResult::timeout()
                    .with_output("satisfied", json!(false))
                    .with_output("actual", json!(last_url)));
            }
            sleep(POLL_INTERVAL).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_equals() {
        let yaml = r#"{ equals: "http://x/done", within: 2s }"#;
        let w: With = serde_yml::from_str(yaml).unwrap();
        let plan = Plan::from_with(w).unwrap();
        assert!(matches!(plan.matcher, Matcher::Equals(ref s) if s == "http://x/done"));
        assert_eq!(plan.timeout, Duration::from_secs(2));
    }

    #[test]
    fn parses_matches_regex() {
        let yaml = r#"{ matches: "^http://x/[a-z]+$" }"#;
        let w: With = serde_yml::from_str(yaml).unwrap();
        let plan = Plan::from_with(w).unwrap();
        assert!(plan.matcher.check("http://x/done"));
        assert!(!plan.matcher.check("http://x/123"));
    }

    #[test]
    fn rejects_both_set_at_parse_time() {
        // Setting both `equals` and `matches` matches neither
        // variant under `untagged + deny_unknown_fields`, so the
        // failure is at `from_str`, before any browser work.
        let yaml = r#"{ equals: "a", matches: "b" }"#;
        let err = serde_yml::from_str::<With>(yaml).unwrap_err();
        // Untagged variants don't carry a single deterministic message
        // when all variants fail; we just assert "no variant matched".
        assert!(
            err.to_string().contains("did not match any variant")
                || err.to_string().contains("data did not match"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_neither_set_at_parse_time() {
        let yaml = r#"{ within: 1s }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn rejects_unknown_field_at_parse_time() {
        let yaml = r#"{ equals: "a", url: "b" }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn rejects_invalid_regex_at_plan_time() {
        let yaml = r#"{ matches: "[bad" }"#;
        let w: With = serde_yml::from_str(yaml).unwrap();
        let err = Plan::from_with(w).unwrap_err();
        assert!(err.to_string().contains("regex"), "got: {err}");
    }
}
