//! `ui/assert-element` — observe an element's existence/visibility.
//!
//! The action is a *waiter*: it polls for the element to reach the
//! requested `expected` state until the deadline, then reports what
//! it saw. Reaching the state within the deadline → `satisfied: true`;
//! deadline elapses → `satisfied: false` and `outcome: Ok` (the
//! observation is conclusive — the element wasn't there). Driver
//! errors that aren't timeout-shaped surface as `ActionError`.
//!
//! Note that `Outcome::Timeout` is reserved for the case where the
//! browser itself times out (page hung, navigation never completes).
//! A successful "we waited and it never appeared" observation is an
//! `Ok` with `satisfied: false` — that's the whole point of expecting
//! `not_exists` / `hidden`.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::field::FieldSource;
use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_TIMEOUT};
use crate::browser::ElementState;
use crate::error::ActionError;
use crate::locator::{ExistenceState, Locator};
use crate::playwright::to_selector;
use crate::with::TimeoutSpec;

#[derive(Debug)]
enum Predicate {
    State(ExistenceState),
    Field {
        source: FieldSource,
        equals: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    #[serde(flatten)]
    source: FieldSource,
    equals: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WithWire {
    locator: Locator,
    #[serde(default)]
    expected: Option<ExistenceState>,
    #[serde(default)]
    expect: Option<Expect>,
    #[serde(default)]
    timeout: Option<TimeoutSpec>,
}

#[derive(Debug)]
struct With {
    locator: Locator,
    predicate: Predicate,
    timeout: Option<TimeoutSpec>,
}

impl<'de> Deserialize<'de> for With {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = WithWire::deserialize(deserializer)?;
        let predicate = match (wire.expected, wire.expect) {
            (Some(state), None) => Predicate::State(state),
            (None, Some(expect)) => Predicate::Field {
                source: expect.source,
                equals: expect.equals,
            },
            _ => {
                return Err(serde::de::Error::custom(
                    "ui/assert-element requires exactly one of `expected` or `expect`",
                ));
            }
        };
        Ok(Self {
            locator: wire.locator,
            predicate,
            timeout: wire.timeout,
        })
    }
}

pub struct AssertElement;

#[async_trait]
impl Action for AssertElement {
    fn uses(&self) -> &'static str {
        "ui/assert-element"
    }

    fn contract(&self) -> crate::action::ActionContract {
        use crate::action::{ActionContract, FieldSpec};
        ActionContract {
            uses: "ui/assert-element",
            summary: "Assert an element reaches an existence/visibility state within a deadline.",
            with: vec![
                FieldSpec::required("locator"),
                FieldSpec::enum_of(
                    "expected",
                    false,
                    &["exists", "not_exists", "visible", "hidden"],
                ),
                FieldSpec::optional("expect"),
                FieldSpec::optional("timeout"),
            ],
            outputs: vec!["satisfied", "count"],
            secret_outputs: vec![],
            example: "- uses: ui/assert-element\n  with: { locator: { role: checkbox, name: \"Receive updates\" }, expect: { field: checked, equals: true } }",
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
                action: "ui/assert-element",
                source: e,
            })?;
        let timeout: Duration = with.timeout.map(Into::into).unwrap_or(DEFAULT_TIMEOUT);
        let selector = to_selector(&with.locator);

        let satisfied = match &with.predicate {
            Predicate::State(expected) => match page
                .wait_for_selector(&selector, map_state(*expected), timeout.as_millis() as f64)
                .await
            {
                Ok(()) => true,
                Err(e) if super::is_timeout_message(&e.to_string()) => false,
                Err(e) => return Err(ActionError::Playwright(format!("ui/assert-element: {e}"))),
            },
            Predicate::Field { source, equals } => {
                let values = page
                    .extract(&selector, source.kind(), source.name())
                    .await
                    .map_err(|e| ActionError::Playwright(format!("ui/assert-element: {e}")))?;
                if values.len() > 1 {
                    return Err(ActionError::Playwright(format!(
                        "ui/assert-element: locator {} matched {} elements; expected exactly one",
                        with.locator.describe(),
                        values.len()
                    )));
                }
                values.first().is_some_and(|actual| actual == equals)
            }
        };

        // `count` reflects matches at observation time. Even when the
        // expectation is `not_exists`/`hidden`, this is the literal
        // post-wait DOM count. A driver-level failure here (page
        // closed, browser crashed) is propagated rather than silently
        // reported as `count = 0` — that would conflate "nothing
        // matched" with "we couldn't ask".
        let count = page
            .count(&selector)
            .await
            .map_err(|e| ActionError::Playwright(format!("ui/assert-element: count: {e}")))?;

        Ok(ActionResult::ok()
            .with_output("satisfied", json!(satisfied))
            .with_output("count", json!(count)))
    }
}

fn map_state(s: ExistenceState) -> ElementState {
    match s {
        ExistenceState::Exists => ElementState::Attached,
        ExistenceState::NotExists => ElementState::Detached,
        ExistenceState::Visible => ElementState::Visible,
        ExistenceState::Hidden => ElementState::Hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assert_element_with() {
        let yaml = r#"
locator: { role: alert, text: "Created" }
expected: visible
timeout: 2s
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        assert!(matches!(
            v.predicate,
            Predicate::State(ExistenceState::Visible)
        ));
        let d: Duration = v.timeout.unwrap().into();
        assert_eq!(d, Duration::from_secs(2));
        assert_eq!(v.locator.text.as_deref(), Some("Created"));
    }

    #[test]
    fn rejects_unknown_expected() {
        let yaml = r#"{ locator: { role: alert }, expected: kind_of_there }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn requires_locator() {
        let yaml = r#"{ expected: visible }"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }
}
