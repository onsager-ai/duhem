//! `ui/select` — choose an option in a `<select>` or `role=combobox`
//! widget. The `by:` discriminator picks one of three Playwright
//! `selectOption` modes: value, label, or zero-based index.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::browser::SelectBy;
use crate::error::ActionError;
use crate::locator::Locator;
use crate::playwright::to_selector;
use crate::with::WithinSpec;

/// `by:` is a tagged union — exactly one of `value`, `label`, `index`
/// is set. `untagged` is intentional: each variant carries its own
/// distinct field name, so YAML stays human-readable
/// (`by: { label: "Admin" }` rather than `by: { kind: label, ... }`).
///
/// `deny_unknown_fields` sits on each variant struct so a mapping that
/// sets two of `value`/`label`/`index` fails to match *any* variant
/// rather than silently picking the first one declared.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum By {
    Value(ByValue),
    Label(ByLabel),
    Index(ByIndex),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ByValue {
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ByLabel {
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ByIndex {
    index: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    locator: Locator,
    by: By,
    #[serde(default)]
    within: Option<WithinSpec>,
}

pub struct Select;

#[async_trait]
impl Action for Select {
    fn uses(&self) -> &'static str {
        "ui/select"
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "ui/select",
                source: e,
            })?;
        let timeout: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);
        let selector = to_selector(&with.locator);

        let by = match with.by {
            By::Value(v) => SelectBy::Value(v.value),
            By::Label(v) => SelectBy::Label(v.label),
            By::Index(v) => SelectBy::Index(v.index as usize),
        };

        match ctx
            .require_page()
            .select_option(&selector, &by, timeout.as_millis() as f64)
            .await
        {
            Ok(()) => Ok(ActionResult::ok()),
            Err(e) if super::is_timeout_message(&e.to_string()) => Ok(ActionResult::timeout()),
            Err(e) => Err(ActionError::Playwright(format!("ui/select: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_select_by_value() {
        let yaml = r#"
locator: { role: combobox, name: Role }
by: { value: admin }
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        match v.by {
            By::Value(ref by) => assert_eq!(by.value, "admin"),
            _ => panic!("expected By::Value"),
        }
    }

    #[test]
    fn parses_select_by_label() {
        let yaml = r#"
locator: { role: combobox, name: Role }
by: { label: "Admin" }
within: 1s
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        match v.by {
            By::Label(ref by) => assert_eq!(by.label, "Admin"),
            _ => panic!("expected By::Label"),
        }
        let d: Duration = v.within.unwrap().into();
        assert_eq!(d, Duration::from_secs(1));
    }

    #[test]
    fn parses_select_by_index() {
        let yaml = r#"
locator: { role: combobox, name: Role }
by: { index: 2 }
"#;
        let v: With = serde_yml::from_str(yaml).unwrap();
        match v.by {
            By::Index(ref by) => assert_eq!(by.index, 2),
            _ => panic!("expected By::Index"),
        }
    }

    #[test]
    fn rejects_select_with_no_by_variant_set() {
        let yaml = r#"
locator: { role: combobox, name: Role }
by: {}
"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn rejects_select_with_multiple_by_variants() {
        // Two fields → no untagged variant alone matches.
        let yaml = r#"
locator: { role: combobox, name: Role }
by: { value: admin, label: "Admin" }
"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn rejects_select_with_unknown_field() {
        let yaml = r#"
locator: { role: combobox }
by: { value: x }
extra: nope
"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    /// `deny_unknown_fields` on each variant must reject the
    /// degenerate three-set case too, not just the two-set case.
    #[test]
    fn rejects_select_with_all_three_by_variants() {
        let yaml = r#"
locator: { role: combobox, name: Role }
by: { value: a, label: "B", index: 1 }
"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    /// Author-controlled YAML: indices outside `u32` range fail
    /// at parse time rather than truncating silently.
    #[test]
    fn rejects_select_with_negative_index() {
        let yaml = r#"
locator: { role: combobox, name: Role }
by: { index: -1 }
"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }

    #[test]
    fn rejects_select_with_index_overflowing_u32() {
        let yaml = r#"
locator: { role: combobox, name: Role }
by: { index: 4294967296 }
"#;
        assert!(serde_yml::from_str::<With>(yaml).is_err());
    }
}
