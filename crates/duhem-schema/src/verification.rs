//! `VerificationDefinition` — the top-level YAML document.
//!
//! Pattern A from `docs/duhem-spec.md` §10.1 (single file, direct
//! execution). The root manifest (`duhem.yml`) and Patterns B/C land
//! in `spec(schema): root manifest v0.1`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::criterion::Criterion;
use crate::project::ProjectDecl;
use crate::provision::Provision;
use crate::step::Step;

/// Named UI locators grouped by page (or any other author-chosen
/// surface such as a modal or navigation bar). Locator bodies stay
/// untyped here; `duhem-actions` owns the authoritative locator schema.
pub type PageCatalog = BTreeMap<String, BTreeMap<String, serde_yml::Value>>;

/// A named, parameterized sequence of steps expanded by the loader.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    /// Optional prose explaining what this reusable sequence is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Per-call parameters. Reuses the input type catalog and
    /// sensitivity marker; values are supplied only by the call site.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, InputDecl>,

    /// Ordered action/flow invocations that make up this flow.
    pub steps: Vec<Step>,

    /// Caller-visible output name to an inner `$steps.*` output.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<String, String>,
}

pub type FlowCatalog = BTreeMap<String, Flow>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationDefinition {
    /// Human-readable name of the verification.
    pub verification: String,

    /// Optional reference to an upstream spec / issue / URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_ref: Option<String>,

    /// Opaque consumer-defined data. Duhem never interprets this map;
    /// it is recorded in run evidence, must never affect a verdict, and
    /// must not count as a change if a diff command is added in future.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "std::collections::BTreeMap<String, serde_json::Value>")]
    pub metadata: BTreeMap<String, serde_yml::Value>,

    /// Optional declared target coordinate (#191): what this
    /// verification verifies (a repo, a service URL, an image, or a
    /// locally-named project). Top rung of the identity-resolution
    /// ladder; absent → the runtime falls through to CI context /
    /// normalized remote / path. A leaf declaration wins over a
    /// manifest-level one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectDecl>,

    /// Optional operator-supplied provisioning lifecycle hooks. When
    /// present, the runtime forks `provision.up:` before `setup:`,
    /// polls `provision.ready:`, and forks `provision.down:`
    /// (if declared) after the criteria loop. Absent → no behavior
    /// change vs setup-only definitions; the wire shape for
    /// `provision:`-less VDs is byte-identical to today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provision: Option<Provision>,

    /// Declared inputs. Map keys are alphabetized on round-trip
    /// (BTreeMap); fixtures should be authored alphabetized.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, InputDecl>,

    /// Leaf-local named locators. When loaded through a manifest these
    /// overlay the composed manifest catalog by page and element name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(
        with = "std::collections::BTreeMap<String, std::collections::BTreeMap<String, serde_json::Value>>"
    )]
    pub pages: PageCatalog,

    /// Leaf-local reusable flows. When loaded through a manifest these
    /// overlay the composed manifest catalog by flow name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flows: FlowCatalog,

    /// Optional setup steps run once before the criteria.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<Step>,

    /// At least one criterion is required (enforced by the validator,
    /// not the type system, so we can produce a friendly error).
    pub criteria: Vec<Criterion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputDecl {
    /// The declared type from the v1 catalog. Unknown names parse-fail
    /// at `from_yaml_str` per the type-catalog spec.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<InputType>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub default: Option<serde_yml::Value>,

    /// Process-environment fallback (specs #346 / #354). This sits
    /// below a selected Duhem profile and above `default:` in
    /// resolution precedence. The declaration may be leaf-local or
    /// suite-wide on a manifest; it never becomes a global override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,

    /// Register the resolved value for evidence/terminal masking. A
    /// secret may not carry a committed `default:`; validation owns that
    /// authoring rule so deserialization can report it alongside other
    /// structural findings.
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,

    /// Pull this name from the parent manifest's declaration and value
    /// resolution ladder. Inherited inputs intentionally omit `type:`
    /// and `default:` because the manifest owns both; `secret: true`
    /// may add protection at the consuming leaf.
    #[serde(default, skip_serializing_if = "is_false")]
    pub inherit: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The closed catalog of declared input types per the type-catalog
/// spec. Wire form is snake_case. Unknown type names parse-fail at
/// `VerificationDefinition::from_yaml_str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    fn rejects_pre_naming_pass_top_level_fields() {
        for old in [
            "environment:\n  up: ./scripts/up.sh\n",
            "inherits: [base_url]\n",
        ] {
            let y = format!("verification: x\n{old}criteria: []\n");
            let err = VerificationDefinition::from_yaml_str(&y).unwrap_err();
            assert!(format!("{err}").contains("unknown field"), "got: {err}");
        }
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
                kind: Some(InputType::String),
                default: Some(serde_yml::Value::String("hi".into())),
                env: None,
                secret: false,
                inherit: false,
            },
        );
        let v = VerificationDefinition {
            verification: "x".into(),
            spec_ref: None,
            metadata: BTreeMap::new(),
            project: None,
            provision: None,
            inputs,
            pages: BTreeMap::new(),
            flows: BTreeMap::new(),
            setup: vec![],
            criteria: vec![],
        };
        let y = v.to_yaml_string().unwrap();
        let back = VerificationDefinition::from_yaml_str(&y).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn metadata_round_trips_opaque_nested_values() {
        let y = r#"
verification: tagged
metadata:
  attempts: 3
  labels: [external, nightly]
  routing:
    owner: platform
    regions: [eu, us]
criteria: []
"#;
        let parsed = VerificationDefinition::from_yaml_str(y).expect("parse metadata map");
        assert_eq!(parsed.metadata["attempts"].as_u64(), Some(3));
        assert_eq!(parsed.metadata["labels"][1].as_str(), Some("nightly"));
        assert_eq!(
            parsed.metadata["routing"]["regions"][0].as_str(),
            Some("eu")
        );

        let round_trip =
            VerificationDefinition::from_yaml_str(&parsed.to_yaml_string().unwrap()).unwrap();
        assert_eq!(round_trip.metadata, parsed.metadata);
    }

    #[test]
    fn absent_metadata_keeps_wire_shape() {
        let parsed = VerificationDefinition::from_yaml_str("verification: x\ncriteria: []\n")
            .expect("parse definition without metadata");
        assert!(parsed.metadata.is_empty());
        assert!(
            !parsed.to_yaml_string().unwrap().contains("metadata:"),
            "an absent optional field must not change the serialized wire shape"
        );
    }

    #[test]
    fn metadata_container_must_be_a_map() {
        for invalid in ["metadata: a string\n", "metadata: [a, b]\n"] {
            let y = format!("verification: x\n{invalid}criteria: []\n");
            let err = VerificationDefinition::from_yaml_str(&y).unwrap_err();
            let message = err.to_string();
            assert!(
                message.contains("invalid type") && message.contains("map"),
                "expected a map-type error for `{invalid}`, got: {message}"
            );
        }
    }

    #[test]
    fn pages_round_trip_and_absent_pages_keep_wire_shape() {
        let absent = VerificationDefinition::from_yaml_str("verification: x\ncriteria: []\n")
            .expect("parse absent");
        assert!(absent.pages.is_empty());
        assert!(
            !absent.to_yaml_string().unwrap().contains("pages:"),
            "an absent additive field must not change the wire shape"
        );

        let y = r#"
verification: catalog
pages:
  login:
    submit: { role: button, name: Sign In }
criteria: []
"#;
        let parsed = VerificationDefinition::from_yaml_str(y).expect("parse pages");
        assert_eq!(
            parsed.pages["login"]["submit"]["name"].as_str(),
            Some("Sign In")
        );
        let round_trip =
            VerificationDefinition::from_yaml_str(&parsed.to_yaml_string().unwrap()).unwrap();
        assert_eq!(parsed, round_trip);
    }

    #[test]
    fn flows_round_trip_and_absent_flows_keep_wire_shape() {
        let absent = VerificationDefinition::from_yaml_str("verification: x\ncriteria: []\n")
            .expect("parse absent");
        assert!(absent.flows.is_empty());
        assert!(!absent.to_yaml_string().unwrap().contains("flows:"));

        let y = r#"
verification: catalog
flows:
  sign_in:
    description: Sign in with supplied credentials
    params:
      password: { type: string, secret: true }
      user: { type: string }
    steps:
      - uses: ui/type
        with: { text: $params.user }
    outputs: {}
criteria: []
"#;
        let parsed = VerificationDefinition::from_yaml_str(y).expect("parse flows");
        assert_eq!(
            parsed.flows["sign_in"].description.as_deref(),
            Some("Sign in with supplied credentials")
        );
        assert!(parsed.flows["sign_in"].params["password"].secret);
        let round_trip =
            VerificationDefinition::from_yaml_str(&parsed.to_yaml_string().unwrap()).unwrap();
        assert_eq!(parsed, round_trip);

        let without_description = r#"verification: catalog
flows:
  sign_in:
    steps:
    - uses: ui/click
criteria: []
"#;
        let parsed_without_description =
            VerificationDefinition::from_yaml_str(without_description).expect("parse old shape");
        assert_eq!(
            parsed_without_description.flows["sign_in"].description,
            None
        );
        assert_eq!(
            parsed_without_description.to_yaml_string().unwrap(),
            without_description,
            "a flow without description must serialize byte-identically to its old wire shape"
        );
    }

    #[test]
    fn round_trip_preserves_inherited_input() {
        let y = r#"
verification: leaf
inputs:
  base_url: { inherit: true }
  region: { inherit: true }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.base_url == "x"
"#;
        let v = VerificationDefinition::from_yaml_str(y).expect("parse");
        assert!(v.inputs["base_url"].inherit);
        assert!(v.inputs["region"].inherit);
        let back = VerificationDefinition::from_yaml_str(&v.to_yaml_string().unwrap()).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn all_catalog_types_parse() {
        for name in ["string", "integer", "number", "boolean", "array", "object"] {
            let y = format!("verification: x\ninputs:\n  k: {{ type: {name} }}\ncriteria: []\n");
            let v = VerificationDefinition::from_yaml_str(&y)
                .unwrap_or_else(|e| panic!("`{name}` should parse: {e}"));
            let decl = v.inputs.get("k").expect("input decl present");
            assert_eq!(decl.kind.expect("type present").as_str(), name);
        }
    }

    #[test]
    fn parses_provision_block() {
        let y = r#"
verification: with-env
provision:
  up: ./scripts/up.sh
  down: ./scripts/down.sh
  ready:
    http:
      url: http://localhost:3000/healthz
      timeout: 60s
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;
        let v = VerificationDefinition::from_yaml_str(y).expect("parse");
        let env = v.provision.expect("provision present");
        assert_eq!(env.up.to_str(), Some("./scripts/up.sh"));
        assert!(env.down.is_some());
        assert!(env.ready.is_some());
    }

    #[test]
    fn provision_without_up_is_a_parse_error() {
        let y = r#"
verification: with-env
provision:
  down: ./scripts/down.sh
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;
        // `up:` is required when `provision:` is present.
        assert!(VerificationDefinition::from_yaml_str(y).is_err());
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
