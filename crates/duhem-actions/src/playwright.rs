//! `Locator` → Playwright selector-string translation.
//!
//! The browser lifecycle (driver, `RunBrowser`, `CheckBrowser`, `Page`)
//! moved to [`crate::browser`] when the driver became the official
//! Playwright Node sidecar (#71). This module keeps the pure,
//! driver-independent selector mapping — and its tests — so the locator
//! grammar is provable in isolation.
//!
//! One *primary strategy* per `Locator` becomes a selector token, and
//! `text`/`scope` compose around it:
//!
//! - `role` → `role=<role>` with `[name="<name>"]` when `name` is set
//!   (exact match — same as `getByRole({ name })`).
//! - `label` → `internal:label="<label>"i` (`getByLabel`, case-insensitive
//!   substring) — reaches inputs with no stable role (`type=password`).
//! - `testid` → `css=[data-testid="<testid>"]` (`getByTestId`, default
//!   attribute; a configurable attribute is a follow-up).
//! - `placeholder` → `internal:attr=[placeholder="<placeholder>"i]`
//!   (`getByPlaceholder`).
//! - `css` → `css=<css>` (raw CSS escape hatch, passed through verbatim).
//! - `text` alone → `internal:text="<text>"i` (`getByText`).
//! - `text` with a non-text primary → a chained `internal:has-text="..."i`
//!   filter step (case-insensitive substring). It is a separate `>>`
//!   step, *not* a pseudo appended to the primary token: Playwright's
//!   role engine rejects `role=alert:has-text(...)` (`#75`).
//!   `internal:has-text` compiles Playwright's `.filter({ hasText })`
//!   and resolves to the matched element (not an inner text node).
//! - `scope` → a `>>` chain: outer first, inner second.
//!
//! Examples:
//!
//! ```text
//! { role: button, name: Create }
//!     → role=button[name="Create"]
//! { label: Password }
//!     → internal:label="Password"i
//! { testid: submit }
//!     → css=[data-testid="submit"]
//! { role: alert, text: "Created" }
//!     → role=alert >> internal:has-text="Created"i
//! { text: "Sign In" }
//!     → internal:text="Sign In"i
//! { role: button, name: Create, scope: { role: list, name: Workspaces } }
//!     → role=list[name="Workspaces"] >> role=button[name="Create"]
//! ```
//!
//! The selector string is handed to the sidecar's `page.locator(...)`,
//! so Playwright's own engines resolve it — no behavior is reimplemented
//! Rust-side. `name`/`text`/`label`/`placeholder`/`testid` strings have
//! `\`, `"` escaped before interpolation; `css` is passed through raw
//! (it *is* a selector). Exotic forms (regex name, exact-vs-substring
//! toggles) remain a follow-up — the strategy set is enforced
//! exactly-one at deserialize in [`crate::locator`].

use crate::locator::Locator;

/// Translate a `Locator` into a Playwright selector string. See the
/// module doc for the mapping rules.
pub fn to_selector(loc: &Locator) -> String {
    let mut chain: Vec<String> = Vec::new();
    collect(loc, &mut chain);
    chain.join(" >> ")
}

fn collect(loc: &Locator, chain: &mut Vec<String>) {
    if let Some(scope) = loc.scope.as_deref() {
        collect(scope, chain);
    }

    // Exactly one primary is guaranteed by `Locator`'s deserialize
    // invariant. `text` is the primary only when no other primary is set.
    let has_named_primary = loc.role.is_some()
        || loc.label.is_some()
        || loc.testid.is_some()
        || loc.placeholder.is_some()
        || loc.css.is_some();

    if let Some(role) = &loc.role {
        let mut s = format!("role={role}");
        if let Some(name) = &loc.name {
            s.push_str(&format!("[name=\"{}\"]", escape_quotes(name)));
        }
        chain.push(s);
    } else if let Some(label) = &loc.label {
        chain.push(format!("internal:label=\"{}\"i", escape_quotes(label)));
    } else if let Some(testid) = &loc.testid {
        chain.push(format!("css=[data-testid=\"{}\"]", escape_quotes(testid)));
    } else if let Some(placeholder) = &loc.placeholder {
        chain.push(format!(
            "internal:attr=[placeholder=\"{}\"i]",
            escape_quotes(placeholder)
        ));
    } else if let Some(css) = &loc.css {
        // Raw CSS escape hatch — passed through verbatim.
        chain.push(format!("css={css}"));
    }

    // `text` is a chained filter on the primary — Playwright's role
    // engine rejects the pseudo form `role=alert:has-text(...)` (#75), so
    // it is its own `>>` step. Standing alone (no named primary) it is
    // itself the primary via `getByText` / `internal:text`.
    if let Some(text) = &loc.text {
        if has_named_primary {
            chain.push(format!("internal:has-text=\"{}\"i", escape_quotes(text)));
        } else {
            chain.push(format!("internal:text=\"{}\"i", escape_quotes(text)));
        }
    }
}

fn escape_quotes(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `role`-primary locator the terse way; other fields default.
    fn role(name: &str) -> Locator {
        Locator {
            role: Some(name.into()),
            ..Default::default()
        }
    }

    #[test]
    fn selector_minimal() {
        assert_eq!(to_selector(&role("button")), "role=button");
    }

    #[test]
    fn selector_role_with_name() {
        let l = Locator {
            role: Some("button".into()),
            name: Some("Create".into()),
            ..Default::default()
        };
        assert_eq!(to_selector(&l), r#"role=button[name="Create"]"#);
    }

    #[test]
    fn selector_role_with_text() {
        let l = Locator {
            role: Some("alert".into()),
            text: Some("Created".into()),
            ..Default::default()
        };
        // `text` is a chained `internal:has-text` step, not a pseudo on
        // the role token — the role engine rejects the latter (#75).
        assert_eq!(
            to_selector(&l),
            r#"role=alert >> internal:has-text="Created"i"#
        );
    }

    #[test]
    fn selector_role_with_name_and_text() {
        let l = Locator {
            role: Some("alert".into()),
            name: Some("Status".into()),
            text: Some("Created".into()),
            ..Default::default()
        };
        assert_eq!(
            to_selector(&l),
            r#"role=alert[name="Status"] >> internal:has-text="Created"i"#
        );
    }

    #[test]
    fn selector_label_primary() {
        let l = Locator {
            label: Some("Password".into()),
            ..Default::default()
        };
        assert_eq!(to_selector(&l), r#"internal:label="Password"i"#);
    }

    #[test]
    fn selector_testid_primary() {
        let l = Locator {
            testid: Some("submit-btn".into()),
            ..Default::default()
        };
        assert_eq!(to_selector(&l), r#"css=[data-testid="submit-btn"]"#);
    }

    #[test]
    fn selector_placeholder_primary() {
        let l = Locator {
            placeholder: Some("Search projects".into()),
            ..Default::default()
        };
        assert_eq!(
            to_selector(&l),
            r#"internal:attr=[placeholder="Search projects"i]"#
        );
    }

    #[test]
    fn selector_css_primary_passthrough() {
        let l = Locator {
            css: Some("div.card > button.primary".into()),
            ..Default::default()
        };
        assert_eq!(to_selector(&l), "css=div.card > button.primary");
    }

    #[test]
    fn selector_standalone_text_is_get_by_text() {
        let l = Locator {
            text: Some("Sign In".into()),
            ..Default::default()
        };
        assert_eq!(to_selector(&l), r#"internal:text="Sign In"i"#);
    }

    #[test]
    fn selector_label_with_text_filter() {
        let l = Locator {
            label: Some("Comments".into()),
            text: Some("draft".into()),
            ..Default::default()
        };
        assert_eq!(
            to_selector(&l),
            r#"internal:label="Comments"i >> internal:has-text="draft"i"#
        );
    }

    #[test]
    fn selector_scope_with_text_on_inner() {
        // text filter chains after the inner role, inside the scope.
        let l = Locator {
            role: Some("alert".into()),
            text: Some("Scoped Created".into()),
            scope: Some(Box::new(Locator {
                role: Some("list".into()),
                name: Some("Workspaces".into()),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(
            to_selector(&l),
            r#"role=list[name="Workspaces"] >> role=alert >> internal:has-text="Scoped Created"i"#
        );
    }

    #[test]
    fn selector_with_scope_outer_first() {
        // §10.3 worked example shape.
        let l = Locator {
            role: Some("button".into()),
            name: Some("Create".into()),
            scope: Some(Box::new(Locator {
                role: Some("list".into()),
                name: Some("Workspaces".into()),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(
            to_selector(&l),
            r#"role=list[name="Workspaces"] >> role=button[name="Create"]"#
        );
    }

    #[test]
    fn selector_recursive_scope() {
        let l = Locator {
            role: Some("button".into()),
            scope: Some(Box::new(Locator {
                role: Some("list".into()),
                scope: Some(Box::new(Locator {
                    role: Some("dialog".into()),
                    name: Some("Settings".into()),
                    ..Default::default()
                })),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(
            to_selector(&l),
            r#"role=dialog[name="Settings"] >> role=list >> role=button"#
        );
    }

    #[test]
    fn selector_quotes_are_escaped() {
        let l = Locator {
            role: Some("button".into()),
            name: Some(r#"Say "hi""#.into()),
            ..Default::default()
        };
        assert_eq!(to_selector(&l), r#"role=button[name="Say \"hi\""]"#);
    }
}
