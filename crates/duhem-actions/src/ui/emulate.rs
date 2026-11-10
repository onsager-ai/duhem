//! `ui/emulate` — set the check's viewport and `prefers-color-scheme`.
//!
//! Applies to the check's own page (its fresh browser context), so the
//! settings are effective for later steps in the same check and never
//! leak into sibling checks or retry attempts. Outputs are read back
//! from the page, not echoed from `with:`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::action::{Action, ActionContract, ActionCtx, ActionResult, FieldSpec};
use crate::error::ActionError;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ColorScheme {
    Light,
    Dark,
    NoPreference,
}

impl ColorScheme {
    fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::NoPreference => "no-preference",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    #[serde(default)]
    viewport: Option<duhem_schema::Viewport>,
    #[serde(default)]
    color_scheme: Option<ColorScheme>,
}

pub struct Emulate;

#[async_trait]
impl Action for Emulate {
    fn uses(&self) -> &'static str {
        "ui/emulate"
    }

    fn contract(&self) -> ActionContract {
        ActionContract {
            uses: "ui/emulate",
            summary: "Set the check's viewport (CSS px) and/or prefers-color-scheme; outputs are the settings read back from the page.",
            with: vec![
                FieldSpec::optional("viewport"),
                FieldSpec::enum_of("color_scheme", false, &["light", "dark", "no-preference"]),
            ],
            outputs: vec!["viewport", "color_scheme"],
            secret_outputs: vec![],
            example: "- id: emu\n  uses: ui/emulate\n  with: { viewport: { width: 390, height: 844 }, color_scheme: dark }",
        }
    }

    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let page = ctx.require_page()?;
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|source| ActionError::InvalidWith {
                action: "ui/emulate",
                source,
            })?;
        if with.viewport.is_none() && with.color_scheme.is_none() {
            return Err(ActionError::Playwright(
                "ui/emulate: set at least one of `viewport` or `color_scheme`".into(),
            ));
        }
        let effective = page
            .emulate(with.viewport, with.color_scheme.map(ColorScheme::as_str))
            .await
            .map_err(|e| ActionError::Playwright(format!("ui/emulate: {e}")))?;
        Ok(ActionResult::ok()
            .with_output(
                "viewport",
                json!({ "width": effective.viewport.width, "height": effective.viewport.height }),
            )
            .with_output("color_scheme", json!(effective.color_scheme)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<With, serde_yml::Error> {
        serde_yml::from_str(yaml)
    }

    #[test]
    fn parses_viewport_and_color_scheme() {
        let w = parse("{ viewport: { width: 390, height: 844 }, color_scheme: dark }").unwrap();
        let v = w.viewport.unwrap();
        assert_eq!((v.width, v.height), (390, 844));
        assert_eq!(w.color_scheme.unwrap().as_str(), "dark");
        let w = parse("{ color_scheme: no-preference }").unwrap();
        assert!(w.viewport.is_none());
        assert_eq!(w.color_scheme.unwrap().as_str(), "no-preference");
    }

    #[test]
    fn rejects_bad_dimensions_schemes_and_unknown_fields() {
        for bad in [
            "{ viewport: { width: 0, height: 10 } }",
            "{ viewport: { width: -1, height: 10 } }",
            "{ viewport: { width: 1.5, height: 10 } }",
            "{ viewport: { width: 10 } }",
            "{ color_scheme: sepia }",
            "{ theme: dark }",
        ] {
            assert!(parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn contract_is_non_judging_and_names_read_back_outputs() {
        let c = Emulate.contract();
        assert!(!c.judges());
        assert_eq!(c.outputs, vec!["viewport", "color_scheme"]);
    }
}
