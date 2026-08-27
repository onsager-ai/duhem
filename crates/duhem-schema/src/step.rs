//! `Step` — one action or reusable-flow invocation inside a check.
//!
//! `uses:` is an opaque string at v0.1; the typed action catalog lands
//! in `spec(actions): ui/* action types v1` and turns this into an
//! enum. `with:` stays untyped (`serde_yml::Value`) until the action
//! catalog gives it a per-action schema. `outputs:` maps a local alias
//! to a runtime extraction path; `secret_outputs:` names scalar output paths
//! that must join the evidence writer's registry before this step emits
//! evidence (spec #355).

use std::collections::BTreeMap;

use schemars::{JsonSchema, SchemaGenerator, schema::Schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ExprStr;

/// Loader-only provenance attached to an action expanded from a
/// reusable flow. It is deliberately skipped on the authored Step wire
/// shape: the evidence crate owns the public event representation, and
/// `duhem resolve --provenance` renders this metadata separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedFlowOrigin {
    pub name: String,
    pub invocation: String,
    pub inner_index: u32,
}

/// Dispatch condition for a step. The three outcome gates retain their
/// original wire form; value expressions reuse the assertion grammar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StepCondition {
    /// Run only while no earlier step in this sequence has failed.
    #[default]
    Success,
    /// Run regardless of an earlier step failure.
    Always,
    /// Run only after an earlier step failure.
    Failure,
    /// Run when the existing expression grammar evaluates to true.
    Expr(ExprStr),
}

impl Serialize for StepCondition {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Success => serializer.serialize_str("success"),
            Self::Always => serializer.serialize_str("always"),
            Self::Failure => serializer.serialize_str("failure"),
            Self::Expr(expr) => expr.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for StepCondition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            "success" => Ok(Self::Success),
            "always" => Ok(Self::Always),
            "failure" => Ok(Self::Failure),
            // A bare word that is not one of the three gates is far more
            // likely a misspelling of them than an expression, and the
            // parser's "expected non-zero digit, '0', '\"', '$', or '('"
            // is useless for that case. Name the real options instead.
            _ => ExprStr::from_source(&raw).map(Self::Expr).map_err(|err| {
                if raw
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    serde::de::Error::custom(format!(
                        "unknown step condition `{raw}` — expected `success`, `always`, \
                         `failure`, or a value expression such as \
                         `$setup.<step>.outputs.<name> > 0`"
                    ))
                } else {
                    serde::de::Error::custom(err)
                }
            }),
        }
    }
}

impl JsonSchema for StepCondition {
    fn schema_name() -> String {
        "StepCondition".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        <String as JsonSchema>::json_schema(generator)
    }
}

impl StepCondition {
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Success)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Reserved for a friendly validation error: fixtures may only be
    /// requested by checks, never by lifecycle steps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// Optional — required only when another assertion or step
    /// references this step via `$steps.<id>.outputs.*`. The
    /// validator enforces that an unreferenced step may omit `id`,
    /// while a referenced step must declare one and that the id is
    /// unique within its check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Optional prose explaining what this action is for. Unlike
    /// [`Step::id`], this is a human-facing display label and is never
    /// used as a reference symbol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Dispatch condition, serialized as `if:`. Omission is
    /// byte-identical to `if: success`.
    #[serde(
        rename = "if",
        default,
        skip_serializing_if = "StepCondition::is_default"
    )]
    pub condition: StepCondition,

    /// Action type identifier (e.g. `ui/click`). Exactly one of this
    /// field and [`Step::call`] is required; the structural validator
    /// owns the friendly exactly-one diagnostic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uses: Option<String>,

    /// Reusable flow name. The loader expands this invocation into
    /// ordinary action steps before the runtime sees the definition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,

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

    /// Raw action-output paths whose resolved scalar values are
    /// sensitive. These are paths, not a second output channel:
    /// `body.data` names the same value available at
    /// `$steps.<id>.outputs.body.data`. Runtime registration rejects
    /// objects and arrays because exact-serialization masking would
    /// give a false impression that the subtree was protected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_outputs: Vec<String>,

    /// Origin of an expanded action step. Not part of the authored VD
    /// schema; the runtime copies it into `StepStarted.flow`.
    #[serde(skip)]
    #[schemars(skip)]
    pub flow: Option<ExpandedFlowOrigin>,

    /// Secret parameter bindings inherited from the enclosing flow
    /// call. Kept off wire and registered before `StepStarted` so
    /// literal credentials receive the same sink-level masking as
    /// declared secret inputs.
    #[serde(skip)]
    #[schemars(skip)]
    pub flow_secrets: Vec<serde_yml::Value>,
}

impl Step {
    /// The catalog action on an executable (already-expanded) step.
    /// Loaders and validators guarantee this before runtime dispatch.
    pub fn uses_name(&self) -> &str {
        self.uses
            .as_deref()
            .expect("executable step must have `uses:` after flow expansion")
    }
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
        assert_eq!(s.uses.as_deref(), Some("ui/click"));
        assert!(s.call.is_none());
        assert!(s.id.is_none());
        assert!(s.description.is_none());
        assert_eq!(s.condition, StepCondition::Success);
        assert!(s.outputs.is_empty());
        assert!(s.secret_outputs.is_empty());
    }

    #[test]
    fn description_is_optional_and_absence_preserves_canonical_bytes() {
        let old_shape = "id: open\nuses: ui/navigate\nwith:\n  url: /login\n";
        let step: Step = serde_yml::from_str(old_shape).expect("parse old shape");
        assert!(step.description.is_none());
        assert_eq!(serde_yml::to_string(&step).expect("serialize"), old_shape);

        let described = "id: open\ndescription: Open the sign-in page\nuses: ui/navigate\n";
        let step: Step = serde_yml::from_str(described).expect("parse description");
        assert_eq!(step.description.as_deref(), Some("Open the sign-in page"));
        assert_eq!(serde_yml::to_string(&step).expect("serialize"), described);
    }

    #[test]
    fn legacy_conditions_and_default_wire_shape_are_unchanged() {
        let old_shape = "uses: ui/click\n";
        let step: Step = serde_yml::from_str(old_shape).expect("parse default");
        assert_eq!(step.condition, StepCondition::Success);
        assert_eq!(serde_yml::to_string(&step).expect("serialize"), old_shape);

        for (wire, expected) in [
            ("success", StepCondition::Success),
            ("always", StepCondition::Always),
            ("failure", StepCondition::Failure),
        ] {
            let step: Step =
                serde_yml::from_str(&format!("if: {wire}\nuses: ui/click\n")).expect(wire);
            assert_eq!(step.condition, expected);
        }
        let expression: Step =
            serde_yml::from_str("if: $setup.existing.outputs.n > 0\nuses: ui/click\n")
                .expect("expression");
        assert!(matches!(expression.condition, StepCondition::Expr(_)));
        assert_eq!(
            serde_yml::to_string(&expression).expect("serialize"),
            "if: $setup.existing.outputs.n > 0\nuses: ui/click\n"
        );
    }

    #[test]
    fn parses_scalar_secret_output_paths() {
        let yaml = r#"
id: login
uses: api/call
with: { method: POST, url: /login }
secret_outputs:
  - body.data
  - body.items[0].key
"#;
        let s: Step = serde_yml::from_str(yaml).expect("parse");
        assert_eq!(s.secret_outputs, ["body.data", "body.items[0].key"]);
        let out = serde_yml::to_string(&s).expect("serialize");
        assert!(out.contains("secret_outputs:\n- body.data\n- body.items[0].key"));
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
    fn rejects_pre_naming_pass_secret_field() {
        let yaml = r#"
uses: api/call
with: { method: GET, url: / }
secret: [body]
"#;
        let err = serde_yml::from_str::<Step>(yaml).unwrap_err();
        assert!(format!("{err}").contains("secret"), "got: {err}");
    }

    #[test]
    fn parses_call_for_validator_to_check() {
        let step: Step = serde_yml::from_str("call: sign_in\n").expect("parse call");
        assert_eq!(step.call.as_deref(), Some("sign_in"));
        assert!(step.uses.is_none());
    }

    #[test]
    fn parses_missing_dispatch_for_validator_to_check() {
        let yaml = "with: {}\n";
        let step = serde_yml::from_str::<Step>(yaml).expect("parse");
        assert!(step.uses.is_none());
        assert!(step.call.is_none());
    }
}
