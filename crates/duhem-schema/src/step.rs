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

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    #[schemars(with = "serde_json::Value")]
    pub with: serde_yml::Value,

    /// Map of local alias → extraction path into the step's raw action
    /// result. Optional: every raw field is already addressable by its
    /// native name (`$steps.<id>.outputs.<field>`), so this is the
    /// escape hatch for the two cases a native name can't cover — a
    /// *rename* (`http_code: status`) and a *derived extraction*
    /// (`project_id: body.data._id`, `first: body.items[0].id`). The
    /// path is opaque at the schema layer; the runtime navigates it
    /// (dotted object keys, `[N]` array indices — spec #273) and records
    /// the value under the alias. Identity (`foo: foo`) is a redundant
    /// no-op the validator lint flags.
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
