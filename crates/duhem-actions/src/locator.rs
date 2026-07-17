//! `Locator` — how UI actions point at an element on the page.
//!
//! Exactly one *primary strategy* selects the element —
//! `role` | `label` | `testid` | `placeholder` | `css` | standalone
//! `text` — mirroring the Playwright `getBy*` family. `name` refines
//! `role` (accessible-name match); `text` refines a non-text primary
//! (`:has-text`) or, alone, is itself the primary (`getByText`);
//! `scope` recursively narrows the search to inside a container. The
//! `role` path is unchanged from the v1 minimal slice — the other
//! strategies are the follow-up promised in `playwright.rs` (label a
//! `type=password` input has no `textbox` role to reach otherwise;
//! testid/css address instrumented or role-less elements).
//!
//! `Locator` is part of the on-the-wire `with:` schema for any UI
//! action that takes one. The primary-strategy invariant is enforced
//! at deserialize (via `LocatorWire` + `TryFrom`), so an ambiguous
//! locator (two primaries, or none) is rejected before `invoke`.

use serde::{Deserialize, Serialize};

/// The address of a DOM element. Resolved against a Playwright `Page`
/// at action time — see `playwright::to_selector`. Exactly one primary
/// strategy is set; the invariant is enforced on deserialize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(try_from = "LocatorWire")]
pub struct Locator {
    /// ARIA role (e.g. `button`, `list`, `alert`). Primary strategy;
    /// pairs with `name`. Optional now that other primaries exist, but
    /// exactly one primary must be present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    /// Associated label text (`getByLabel`) — reaches inputs with no
    /// stable role, e.g. `<input type="password">`. Primary strategy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Test id (`getByTestId`, default `data-testid` attribute).
    /// Primary strategy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub testid: Option<String>,

    /// Placeholder text (`getByPlaceholder`). Primary strategy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// Raw CSS selector escape hatch. Primary strategy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub css: Option<String>,

    /// Accessible name. Refines `role` (exact match); requires `role`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Substring text content match (case-insensitive). Combined with a
    /// non-text primary via Playwright's `:has-text()`; standalone it is
    /// the primary strategy (`getByText`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// Parent locator. The element is searched only inside an element
    /// that itself matches `scope`. Recursive — `scope` may itself
    /// have a `scope`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Box<Locator>>,
}

/// Wire form of [`Locator`]: all fields optional, unknown fields
/// rejected. `TryFrom` enforces the exactly-one-primary invariant so
/// [`Locator`] values in the rest of the crate are always well-formed.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocatorWire {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    testid: Option<String>,
    #[serde(default)]
    placeholder: Option<String>,
    #[serde(default)]
    css: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    scope: Option<Box<Locator>>,
}

impl Locator {
    /// The exactly-one-primary (+ `name` requires `role`) invariant.
    /// Shared by the deserialize path (`LocatorWire`) and inline consumers
    /// that assemble a `Locator` field-by-field (`ui/click`), so every
    /// `Locator` in the crate is well-formed however it was built.
    pub(crate) fn validate_primary(&self) -> Result<(), String> {
        let set: Vec<&str> = [
            ("role", self.role.is_some()),
            ("label", self.label.is_some()),
            ("testid", self.testid.is_some()),
            ("placeholder", self.placeholder.is_some()),
            ("css", self.css.is_some()),
        ]
        .into_iter()
        .filter(|(_, present)| *present)
        .map(|(k, _)| k)
        .collect();
        if set.len() > 1 {
            return Err(format!(
                "locator has multiple primary strategies ({}); use exactly one of role/label/testid/placeholder/css",
                set.join(", ")
            ));
        }
        // No named primary is allowed only when `text` alone stands in as
        // the primary (getByText).
        if set.is_empty() && self.text.is_none() {
            return Err(
                "locator needs a primary strategy: one of role/label/testid/placeholder/css, or a standalone text"
                    .to_string(),
            );
        }
        if self.name.is_some() && self.role.is_none() {
            return Err("locator `name` refines `role` and requires a `role`".to_string());
        }
        Ok(())
    }
}

impl TryFrom<LocatorWire> for Locator {
    type Error = String;

    fn try_from(w: LocatorWire) -> Result<Self, Self::Error> {
        let loc = Locator {
            role: w.role,
            label: w.label,
            testid: w.testid,
            placeholder: w.placeholder,
            css: w.css,
            name: w.name,
            text: w.text,
            scope: w.scope,
        };
        loc.validate_primary()?;
        Ok(loc)
    }
}

/// Existence/visibility expectation for `ui/assert-element`.
///
/// Closed enum — the four states map directly onto Playwright's
/// `WaitForSelectorState` plus DOM presence semantics, and adding
/// new states is a schema change handled by a follow-up spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistenceState {
    /// Element is present in the DOM (visible or not).
    Exists,
    /// Element is not present in the DOM.
    NotExists,
    /// Element is present and visually rendered (non-zero size,
    /// `display != none`, `visibility != hidden`).
    Visible,
    /// Element is present in the DOM but not visible.
    Hidden,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_locator() {
        let yaml = r#"role: button"#;
        let l: Locator = serde_yml::from_str(yaml).unwrap();
        assert_eq!(l.role.as_deref(), Some("button"));
        assert!(l.name.is_none());
        assert!(l.text.is_none());
        assert!(l.scope.is_none());
    }

    #[test]
    fn parses_locator_with_name_and_text() {
        let yaml = r#"{ role: alert, text: "Created" }"#;
        let l: Locator = serde_yml::from_str(yaml).unwrap();
        assert_eq!(l.role.as_deref(), Some("alert"));
        assert_eq!(l.text.as_deref(), Some("Created"));
    }

    #[test]
    fn parses_label_primary() {
        let l: Locator = serde_yml::from_str(r#"label: Password"#).unwrap();
        assert_eq!(l.label.as_deref(), Some("Password"));
        assert!(l.role.is_none());
    }

    #[test]
    fn parses_testid_css_placeholder_primaries() {
        let t: Locator = serde_yml::from_str(r#"testid: submit-btn"#).unwrap();
        assert_eq!(t.testid.as_deref(), Some("submit-btn"));
        let c: Locator = serde_yml::from_str(r#"{ css: "div.card > button" }"#).unwrap();
        assert_eq!(c.css.as_deref(), Some("div.card > button"));
        let p: Locator = serde_yml::from_str(r#"placeholder: Search"#).unwrap();
        assert_eq!(p.placeholder.as_deref(), Some("Search"));
    }

    #[test]
    fn parses_standalone_text_primary() {
        let l: Locator = serde_yml::from_str(r#"text: "Sign In""#).unwrap();
        assert_eq!(l.text.as_deref(), Some("Sign In"));
        assert!(l.role.is_none());
    }

    #[test]
    fn rejects_multiple_primaries() {
        let err =
            serde_yml::from_str::<Locator>(r#"{ role: button, label: Password }"#).unwrap_err();
        assert!(
            format!("{err}").contains("multiple primary strategies"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_no_primary() {
        let err = serde_yml::from_str::<Locator>(r#"{ scope: { role: list } }"#).unwrap_err();
        assert!(
            format!("{err}").contains("needs a primary strategy"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_name_without_role() {
        let err = serde_yml::from_str::<Locator>(r#"{ label: Password, name: pw }"#).unwrap_err();
        assert!(format!("{err}").contains("requires a `role`"), "got: {err}");
    }

    #[test]
    fn parses_recursive_scope_from_spec_worked_example() {
        // §10.3 worked-example shape.
        let yaml = r#"
role: button
name: Create
scope:
  role: list
  name: Workspaces
"#;
        let l: Locator = serde_yml::from_str(yaml).unwrap();
        assert_eq!(l.role.as_deref(), Some("button"));
        assert_eq!(l.name.as_deref(), Some("Create"));
        let scope = l.scope.expect("scope present");
        assert_eq!(scope.role.as_deref(), Some("list"));
        assert_eq!(scope.name.as_deref(), Some("Workspaces"));
        assert!(scope.scope.is_none());
    }

    #[test]
    fn parses_doubly_nested_scope() {
        let yaml = r#"
role: button
scope:
  role: list
  scope:
    role: dialog
    name: Settings
"#;
        let l: Locator = serde_yml::from_str(yaml).unwrap();
        let outer_scope = l.scope.expect("outer");
        let inner_scope = outer_scope.scope.expect("inner");
        assert_eq!(inner_scope.role.as_deref(), Some("dialog"));
        assert_eq!(inner_scope.name.as_deref(), Some("Settings"));
    }

    #[test]
    fn rejects_unknown_field() {
        let err = serde_yml::from_str::<Locator>(r#"{ role: button, color: red }"#).unwrap_err();
        assert!(format!("{err}").contains("unknown field"), "got: {err}");
    }

    #[test]
    fn existence_state_serializes_snake_case() {
        assert_eq!(
            serde_yml::to_string(&ExistenceState::NotExists)
                .unwrap()
                .trim(),
            "not_exists"
        );
        let s: ExistenceState = serde_yml::from_str("visible").unwrap();
        assert_eq!(s, ExistenceState::Visible);
    }
}
