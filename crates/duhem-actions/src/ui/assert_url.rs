//! `ui/assert-url` — observe the current page URL.
//!
//! Like `ui/assert-element`, this is a *waiter*: it polls the URL
//! until the matcher succeeds or the deadline elapses. Matcher
//! success within the deadline → `satisfied: true`. Deadline
//! elapses with the URL never matching → `Outcome::Timeout`
//! per the spec on issue #37 (the URL never reaching the
//! expectation is "we didn't get to where we said we would",
//! which is the timeout-shaped outcome the judge maps to
//! `Inconclusive(Timeout)`).
//!
//! Exactly one of `equals:` (literal string match) or `matches:`
//! (regex) is required; the validator rejects both-set and
//! neither-set at deserialize time.

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    #[serde(default)]
    equals: Option<String>,
    #[serde(default)]
    matches: Option<String>,
    #[serde(default)]
    within: Option<WithinSpec>,
}

#[derive(Debug)]
enum Matcher {
    Equals(String),
    Matches(Regex),
}

impl Matcher {
    fn from_with(w: &With) -> Result<Self, ActionError> {
        match (&w.equals, &w.matches) {
            (Some(_), Some(_)) => Err(ActionError::InvalidWith {
                action: "ui/assert-url",
                source: serde::de::Error::custom(
                    "exactly one of `equals:` or `matches:` is required; both were set",
                ),
            }),
            (None, None) => Err(ActionError::InvalidWith {
                action: "ui/assert-url",
                source: serde::de::Error::custom(
                    "exactly one of `equals:` or `matches:` is required; neither was set",
                ),
            }),
            (Some(s), None) => Ok(Matcher::Equals(s.clone())),
            (None, Some(pat)) => {
                let re = Regex::new(pat).map_err(|e| ActionError::InvalidWith {
                    action: "ui/assert-url",
                    source: serde::de::Error::custom(format!("invalid regex: {e}")),
                })?;
                Ok(Matcher::Matches(re))
            }
        }
    }

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

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "ui/assert-url",
                source: e,
            })?;
        let timeout: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);
        let matcher = Matcher::from_with(&with)?;

        let started = Instant::now();
        loop {
            let last_url = ctx
                .page
                .url()
                .map_err(|e| ActionError::Playwright(format!("ui/assert-url: url: {e}")))?;
            if matcher.check(&last_url) {
                return Ok(ActionResult::ok()
                    .with_output("satisfied", json!(true))
                    .with_output("actual", json!(last_url)));
            }
            if started.elapsed() >= timeout {
                let mut r = ActionResult::timeout();
                r.outputs.insert("satisfied".into(), json!(false));
                r.outputs.insert("actual".into(), json!(last_url));
                return Ok(r);
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
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert_eq!(v.equals.as_deref(), Some("http://x/done"));
        let m = Matcher::from_with(&v).unwrap();
        assert!(matches!(m, Matcher::Equals(_)));
    }

    #[test]
    fn parses_matches_regex() {
        let yaml = r#"{ matches: "^http://x/[a-z]+$" }"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        let m = Matcher::from_with(&v).unwrap();
        assert!(m.check("http://x/done"));
        assert!(!m.check("http://x/123"));
    }

    #[test]
    fn rejects_both_set() {
        let yaml = r#"{ equals: "a", matches: "b" }"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        let err = Matcher::from_with(&v).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("both"), "got: {msg}");
    }

    #[test]
    fn rejects_neither_set() {
        let yaml = r#"{ within: 1s }"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        let err = Matcher::from_with(&v).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("neither"), "got: {msg}");
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = r#"{ equals: "a", url: "b" }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn rejects_invalid_regex() {
        let yaml = r#"{ matches: "[bad" }"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        let err = Matcher::from_with(&v).unwrap_err();
        assert!(err.to_string().contains("regex"), "got: {err}");
    }
}
