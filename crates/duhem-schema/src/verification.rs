//! `VerificationDefinition` — the top-level YAML document.
//!
//! Pattern A from `docs/duhem-spec.md` §10.1 (single file, direct
//! execution). The root manifest (`duhem.yml`) and Patterns B/C land
//! in `spec(schema): root manifest v0.1`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::criterion::Criterion;
use crate::step::Step;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationDefinition {
    /// Human-readable name of the verification.
    pub verification: String,

    /// Optional reference to an upstream spec / issue / URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_ref: Option<String>,

    /// Declared inputs. Map keys are alphabetized on round-trip
    /// (BTreeMap); fixtures should be authored alphabetized.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, InputDecl>,

    /// Optional setup steps run once before the criteria.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<Step>,

    /// At least one criterion is required (enforced by the validator,
    /// not the type system, so we can produce a friendly error).
    pub criteria: Vec<Criterion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDecl {
    /// The declared type from the v1 catalog. Unknown names parse-fail
    /// at `from_yaml_str` per the type-catalog spec.
    #[serde(rename = "type")]
    pub kind: InputType,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_yml::Value>,
}

/// The closed catalog of declared input types per the type-catalog
/// spec. Wire form is snake_case. Unknown type names parse-fail at
/// `VerificationDefinition::from_yaml_str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputType {
    String,
    Integer,
    Number,
    Boolean,
    Array,
    Object,
}

impl InputType {
    /// Snake-case wire form. Matches the `serde(rename_all)` above so
    /// error messages and validation diagnostics speak the same names
    /// authors wrote.
    pub fn as_str(self) -> &'static str {
        match self {
            InputType::String => "string",
            InputType::Integer => "integer",
            InputType::Number => "number",
            InputType::Boolean => "boolean",
            InputType::Array => "array",
            InputType::Object => "object",
        }
    }
}

impl std::fmt::Display for InputType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Top-level errors from loading a Verification Definition off the
/// wire. Validation errors are reported separately by `validate()` so
/// callers can distinguish "this YAML is malformed" from "this YAML
/// parses but violates a structural rule".
#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yml::Error),
}

impl SchemaError {
    /// Source location (line/column) of the parse failure, if the
    /// underlying error carries one. Lets callers render errors with
    /// editor-friendly provenance without matching on the variant.
    pub fn location(&self) -> Option<serde_yml::Location> {
        match self {
            SchemaError::Yaml(e) => e.location(),
        }
    }
}

impl VerificationDefinition {
    /// Parse a Verification Definition from YAML source. Does not run
    /// the structural validator; call `crate::validate()` for that.
    pub fn from_yaml_str(src: &str) -> Result<Self, SchemaError> {
        serde_yml::from_str(src).map_err(SchemaError::from)
    }

    /// Re-emit a parsed Verification Definition as YAML. Order is
    /// preserved for `criteria` / `checks` / `steps` (Vec); `inputs`
    /// is alphabetized by key (BTreeMap).
    pub fn to_yaml_string(&self) -> Result<String, SchemaError> {
        serde_yml::to_string(self).map_err(SchemaError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_definition() {
        let y = r#"
verification: minimal
criteria:
  - id: AC-1
    description: trivial
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.x == 1
"#;
        let v = VerificationDefinition::from_yaml_str(y).expect("parse");
        assert_eq!(v.verification, "minimal");
        assert_eq!(v.criteria.len(), 1);
        assert!(v.inputs.is_empty());
        assert!(v.setup.is_empty());
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let y = "verification: x\nfoo: bar\ncriteria: []\n";
        let err = VerificationDefinition::from_yaml_str(y).unwrap_err();
        assert!(format!("{err}").contains("unknown field"), "got: {err}");
    }

    #[test]
    fn yaml_error_carries_location() {
        // Tab where YAML expects spaces is one common source of error
        // with a real line/column.
        let y = "verification: x\ncriteria:\n\t- id: AC-1\n";
        let err = VerificationDefinition::from_yaml_str(y).unwrap_err();
        assert!(err.location().is_some(), "expected location info: {err}");
    }

    #[test]
    fn round_trip_preserves_input_decl() {
        let mut inputs = BTreeMap::new();
        inputs.insert(
            "name".into(),
            InputDecl {
                kind: InputType::String,
                default: Some(serde_yml::Value::String("hi".into())),
            },
        );
        let v = VerificationDefinition {
            verification: "x".into(),
            spec_ref: None,
            inputs,
            setup: vec![],
            criteria: vec![],
        };
        let y = v.to_yaml_string().unwrap();
        let back = VerificationDefinition::from_yaml_str(&y).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn all_catalog_types_parse() {
        for name in ["string", "integer", "number", "boolean", "array", "object"] {
            let y = format!("verification: x\ninputs:\n  k: {{ type: {name} }}\ncriteria: []\n");
            let v = VerificationDefinition::from_yaml_str(&y)
                .unwrap_or_else(|e| panic!("`{name}` should parse: {e}"));
            let decl = v.inputs.get("k").expect("input decl present");
            assert_eq!(decl.kind.as_str(), name);
        }
    }

    #[test]
    fn unknown_type_name_is_parse_error() {
        let y = r#"
verification: x
inputs:
  k: { type: bogus }
criteria: []
"#;
        let err = VerificationDefinition::from_yaml_str(y).unwrap_err();
        assert!(err.location().is_some(), "expected location info: {err}");
        let msg = format!("{err}");
        assert!(
            msg.contains("bogus") || msg.contains("unknown variant"),
            "expected variant error mentioning `bogus`, got: {msg}"
        );
    }
}
