//! `Locator` — how UI actions point at an element on the page.
//!
//! The shape mirrors Playwright's `getByRole` semantics (role + name +
//! optional substring text) plus a recursive `scope` for "inside this
//! container, find that element". Recursive scoping matches the
//! `docs/duhem-spec.md` §10.3 worked example
//! (`scope: { role: "list", name: "Workspaces" }`).
//!
//! `Locator` is part of the on-the-wire `with:` schema for any UI
//! action that takes one. It is a stable contract across the `ui/*`
//! catalog (`spec(actions): ui/* action types v1`).

use serde::{Deserialize, Serialize};

/// The address of a DOM element. Resolved against a Playwright `Page`
/// at action time — see `playwright::to_selector`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    /// ARIA role (e.g. `button`, `list`, `alert`). Required — name +
    /// text alone are not specific enough to be a stable address.
    pub role: String,

    /// Accessible name. Optional — used as an exact-match attribute
    /// when present, omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Substring text content match. Optional — combined with `role`
    /// via Playwright's `:has-text()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// Parent locator. The element is searched only inside an element
    /// that itself matches `scope`. Recursive — `scope` may itself
    /// have a `scope`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Box<Locator>>,
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
        assert_eq!(l.role, "button");
        assert!(l.name.is_none());
        assert!(l.text.is_none());
        assert!(l.scope.is_none());
    }

    #[test]
    fn parses_locator_with_name_and_text() {
        let yaml = r#"{ role: alert, text: "Created" }"#;
        let l: Locator = serde_yml::from_str(yaml).unwrap();
        assert_eq!(l.role, "alert");
        assert_eq!(l.text.as_deref(), Some("Created"));
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
        assert_eq!(l.role, "button");
        assert_eq!(l.name.as_deref(), Some("Create"));
        let scope = l.scope.expect("scope present");
        assert_eq!(scope.role, "list");
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
        assert_eq!(inner_scope.role, "dialog");
        assert_eq!(inner_scope.name.as_deref(), Some("Settings"));
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = r#"{ role: button, color: red }"#;
        let err = serde_yml::from_str::<Locator>(yaml).unwrap_err();
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
