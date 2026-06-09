//! `Locator` → Playwright selector-string translation.
//!
//! The browser lifecycle (driver, `RunBrowser`, `CheckBrowser`, `Page`)
//! moved to [`crate::browser`] when the driver became the official
//! Playwright Node sidecar (#71). This module keeps the pure,
//! driver-independent selector mapping — and its tests — verbatim, so
//! the locator grammar is provably unchanged across the migration.
//!
//! `Locator { role, name?, text?, scope? }` translates into
//! Playwright's selector engine syntax:
//!
//! - Base: `role=<role>` with `[name="<name>"]` when `name` is set
//!   (exact match — same as `getByRole({ name })`).
//! - `text` becomes a `:has-text("...")` substring filter.
//! - `scope` becomes a `>>` chain: outer first, inner second.
//!
//! Examples:
//!
//! ```text
//! { role: button, name: Create }
//!     → role=button[name="Create"]
//! { role: alert, text: "Created" }
//!     → role=alert:has-text("Created")
//! { role: button, name: Create, scope: { role: list, name: Workspaces } }
//!     → role=list[name="Workspaces"] >> role=button[name="Create"]
//! ```
//!
//! The selector string is handed to the sidecar's `page.locator(...)`,
//! so Playwright's own role / accessible-name / `:has-text` engines
//! resolve it — no behavior is reimplemented Rust-side.
//!
//! Quoting: `name`/`text` strings have `\`, `"` escaped before
//! interpolation. Anything more exotic (regex name, partial text via
//! Playwright's `getByText` form) is deliberately not supported in the
//! v1 minimal slice — same trait, follow-up spec.

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
    let mut s = format!("role={}", loc.role);
    if let Some(name) = &loc.name {
        s.push_str(&format!("[name=\"{}\"]", escape_quotes(name)));
    }
    if let Some(text) = &loc.text {
        s.push_str(&format!(":has-text(\"{}\")", escape_quotes(text)));
    }
    chain.push(s);
}

fn escape_quotes(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_minimal() {
        let l = Locator {
            role: "button".into(),
            name: None,
            text: None,
            scope: None,
        };
        assert_eq!(to_selector(&l), "role=button");
    }

    #[test]
    fn selector_role_with_name() {
        let l = Locator {
            role: "button".into(),
            name: Some("Create".into()),
            text: None,
            scope: None,
        };
        assert_eq!(to_selector(&l), r#"role=button[name="Create"]"#);
    }

    #[test]
    fn selector_role_with_text() {
        let l = Locator {
            role: "alert".into(),
            name: None,
            text: Some("Created".into()),
            scope: None,
        };
        assert_eq!(to_selector(&l), r#"role=alert:has-text("Created")"#);
    }

    #[test]
    fn selector_with_scope_outer_first() {
        // §10.3 worked example shape.
        let l = Locator {
            role: "button".into(),
            name: Some("Create".into()),
            text: None,
            scope: Some(Box::new(Locator {
                role: "list".into(),
                name: Some("Workspaces".into()),
                text: None,
                scope: None,
            })),
        };
        assert_eq!(
            to_selector(&l),
            r#"role=list[name="Workspaces"] >> role=button[name="Create"]"#
        );
    }

    #[test]
    fn selector_recursive_scope() {
        let l = Locator {
            role: "button".into(),
            name: None,
            text: None,
            scope: Some(Box::new(Locator {
                role: "list".into(),
                name: None,
                text: None,
                scope: Some(Box::new(Locator {
                    role: "dialog".into(),
                    name: Some("Settings".into()),
                    text: None,
                    scope: None,
                })),
            })),
        };
        assert_eq!(
            to_selector(&l),
            r#"role=dialog[name="Settings"] >> role=list >> role=button"#
        );
    }

    #[test]
    fn selector_quotes_are_escaped() {
        let l = Locator {
            role: "button".into(),
            name: Some(r#"Say "hi""#.into()),
            text: None,
            scope: None,
        };
        assert_eq!(to_selector(&l), r#"role=button[name="Say \"hi\""]"#);
    }
}
