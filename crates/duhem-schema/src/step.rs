//! `Step` — one action invocation inside a check's execution sequence.
//!
//! `uses:` is an opaque string at v0.1; the typed action catalog lands
//! in `spec(actions): ui/* action types v1` and turns this into an
//! enum. `with:` stays untyped (`serde_yml::Value`) until the action
//! catalog gives it a per-action schema. `outputs:` maps an output
//! name (referenced as `$steps.<step_id>.outputs.<name>`) to a runtime
//! extraction expression — a string here, evaluated in the runtime
//! spec.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Optional — required only when another assertion or step
    /// references this step via `$steps.<id>.outputs.*`. The
    /// validator enforces that an unreferenced step may omit `id`,
    /// while a referenced step must declare one and that the id is
    /// unique within its check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Action type identifier (e.g. `ui/click`). At v0.1 this is any
    /// non-empty string; the catalog spec types it later.
    pub uses: String,

    /// Action-specific arguments. Untyped at the schema layer; the
    /// per-action `with:` schema lives with the action implementation.
    #[serde(default, skip_serializing_if = "is_null")]
    pub with: serde_yml::Value,

    /// Map of output name → extraction expression. Extraction
    /// expressions are opaque strings here (e.g. `response.status`);
    /// the runtime evaluator binds them to live values during
    /// execution.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<String, String>,
}

fn is_null(v: &serde_yml::Value) -> bool {
    matches!(v, serde_yml::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_step() {
        let yaml = r#"
uses: ui/click
with: { role: button, name: Create }
"#;
        let s: Step = serde_yml::from_str(yaml).expect("parse");
        assert_eq!(s.uses, "ui/click");
        assert!(s.id.is_none());
        assert!(s.outputs.is_empty());
    }

    #[test]
    fn parses_step_with_outputs() {
        let yaml = r#"
id: api_call
uses: api/observe
with: { method: POST }
outputs:
  status: response.status
  body: response.body
"#;
        let s: Step = serde_yml::from_str(yaml).expect("parse");
        assert_eq!(s.id.as_deref(), Some("api_call"));
        assert_eq!(s.outputs.len(), 2);
        assert_eq!(s.outputs["status"], "response.status");
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = r#"
uses: ui/click
with: {}
extra: nope
"#;
        let err = serde_yml::from_str::<Step>(yaml).unwrap_err();
        assert!(format!("{err}").contains("unknown field"), "got: {err}");
    }

    #[test]
    fn rejects_missing_uses() {
        let yaml = "with: {}\n";
        assert!(serde_yml::from_str::<Step>(yaml).is_err());
    }
}
