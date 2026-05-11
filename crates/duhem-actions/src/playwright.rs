//! Playwright lifecycle + `Locator` → selector translation.
//!
//! ## Lifecycle
//!
//! - One `Playwright` driver + one `Browser` per `duhem run` (held by
//!   `RunBrowser`).
//! - One `BrowserContext` + one `Page` per check (held by
//!   `CheckBrowser`). Cookies and storage are isolated per check —
//!   the "fresh user" intuition.
//! - Headless by default; `--headed` on `duhem run` flips
//!   `RunBrowser::launch(headed: true)`. The CLI spec wires the flag.
//!
//! The Playwright Node driver itself ships inside the `playwright`
//! crate, but the *browser binary* is the user's responsibility:
//! `npx playwright install chromium`. `RunBrowser::launch` translates
//! the missing-binary error into a clear hint instead of dumping the
//! raw driver stderr — auto-install is deliberately out of scope
//! (multi-hundred-MB download with no opt-in; see Design /
//! "browser-binary install" alignment item).
//!
//! ## Locator → Playwright selector
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
//! Quoting: `name`/`text` strings have `\`, `"` escaped before
//! interpolation. Anything more exotic (regex name, partial text via
//! Playwright's `getByText` form) is deliberately not supported in
//! the v1 minimal slice — same trait, follow-up spec.

use playwright::Playwright;
use playwright::api::{Browser, BrowserContext, Page};

use crate::error::ActionError;
use crate::locator::Locator;

/// Driver + browser shared for the lifetime of a `duhem run`. Drop to
/// release.
pub struct RunBrowser {
    _playwright: Playwright,
    pub browser: Browser,
}

impl RunBrowser {
    /// Launch chromium. `headed = false` (the default) runs without
    /// a visible window. Translates the common "browser binary
    /// missing" error into an `ActionError::Playwright` carrying the
    /// exact `npx` command to run.
    pub async fn launch(headed: bool) -> Result<Self, ActionError> {
        let playwright = Playwright::initialize()
            .await
            .map_err(|e| ActionError::Playwright(format!("driver init: {e}")))?;
        let chromium = playwright.chromium();
        let browser = chromium
            .launcher()
            .headless(!headed)
            .launch()
            .await
            .map_err(|e| ActionError::Playwright(humanize_launch_error(&e.to_string())))?;
        Ok(Self {
            _playwright: playwright,
            browser,
        })
    }

    /// Allocate a fresh context + page for one check. Caller owns the
    /// returned handle and drops it at `check_finished`.
    pub async fn open_check(&self) -> Result<CheckBrowser, ActionError> {
        let context = self
            .browser
            .context_builder()
            .build()
            .await
            .map_err(|e| ActionError::Playwright(format!("context: {e}")))?;
        let page = context
            .new_page()
            .await
            .map_err(|e| ActionError::Playwright(format!("page: {e}")))?;
        Ok(CheckBrowser { context, page })
    }
}

/// Per-check Playwright handle. `context` is held alongside `page`
/// because dropping the context implicitly tears down the page.
pub struct CheckBrowser {
    pub context: BrowserContext,
    pub page: Page,
}

impl CheckBrowser {
    /// Explicitly close the context. Drop does the same; this is for
    /// callers that want the close-failure surfaced.
    pub async fn close(self) -> Result<(), ActionError> {
        self.context
            .close()
            .await
            .map_err(|e| ActionError::Playwright(format!("close: {e}")))
    }
}

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

/// Recognize the Playwright "browser binary missing" failure mode
/// and emit the install command. Other errors pass through verbatim.
fn humanize_launch_error(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("executable doesn't exist")
        || lower.contains("install missing dependencies")
        || lower.contains("browsertype.launch")
    {
        format!(
            "chromium binary not installed. Run `npx playwright install chromium` once and retry. (driver said: {raw})"
        )
    } else {
        raw.to_string()
    }
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
