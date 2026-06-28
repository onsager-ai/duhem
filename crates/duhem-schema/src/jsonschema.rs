//! JSON Schema emission for the Verification-Definition / root-manifest
//! YAML shapes (spec issue #133).
//!
//! Most schema types `#[derive(JsonSchema)]`, which honors their
//! existing `#[serde(...)]` attributes (`deny_unknown_fields` →
//! `additionalProperties: false`, enum reprs, field renames). Four
//! types carry hand-written `Serialize`/`Deserialize` impls whose wire
//! shape `schemars` cannot infer from a derive; this module supplies a
//! manual `JsonSchema` for each. The goal is editor autocomplete +
//! not red-squiggling valid input, not perfect fidelity — so these
//! impls are deliberately useful-but-permissive.
//!
//! `json_schema()` is the public entry point: it builds one combined
//! schema (a `oneOf` of `VerificationDefinition` and `RootManifest`)
//! and serializes it to a `serde_json::Value`, ready to write to
//! `schema/duhem.schema.json`.
//
// budget-allow: manual JsonSchema impls for the four hand-written-serde
// types live here by design (keeps the type files under the file budget
// and the orphan rule lets us impl the foreign trait for local types
// from this module).

use schemars::JsonSchema;
use schemars::r#gen::SchemaGenerator;
use schemars::schema::{
    InstanceType, Metadata, ObjectValidation, RootSchema, Schema, SchemaObject, SubschemaValidation,
};

use crate::assertion::Assertion;
use crate::environment::{DurationSpec, HttpReadyProbe, ReadyProbe};
use crate::expr::ExprStr;
use crate::manifest::RootManifest;
use crate::verification::VerificationDefinition;

/// `DurationSpec` accepts either an integer (milliseconds) or a
/// suffixed string (`60s`, `2m`). Permissive `oneOf`.
impl JsonSchema for DurationSpec {
    fn schema_name() -> String {
        "DurationSpec".to_string()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        let string = Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            ..Default::default()
        });
        let integer = Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Integer.into()),
            ..Default::default()
        });
        Schema::Object(SchemaObject {
            subschemas: Some(Box::new(SubschemaValidation {
                one_of: Some(vec![string, integer]),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

/// An `ExprStr` is an authored expression — always a string on the
/// wire; the parse happens after deserialization.
impl JsonSchema for ExprStr {
    fn schema_name() -> String {
        "ExprStr".to_string()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            ..Default::default()
        })
    }
}

/// An `Assertion` is either a bare expression (string) or a single-key
/// mapping (`type_check`, `matches`, `in`, `exists`, `equal`). The
/// JSON Schema layer stays permissive — the object branch is any
/// object — and `duhem validate` owns the precise per-form check.
impl JsonSchema for Assertion {
    fn schema_name() -> String {
        "Assertion".to_string()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        let string = Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            ..Default::default()
        });
        let object = Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Object.into()),
            ..Default::default()
        });
        Schema::Object(SchemaObject {
            subschemas: Some(Box::new(SubschemaValidation {
                one_of: Some(vec![string, object]),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

/// `ready:` is a single-kind probe map. v1 ships `http:` only; the
/// hand-written serde rejects unknown keys and the missing-kind case,
/// so the schema models it as an object with one optional `http`
/// property referencing [`HttpReadyProbe`].
impl JsonSchema for ReadyProbe {
    fn schema_name() -> String {
        "ReadyProbe".to_string()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let http = generator.subschema_for::<HttpReadyProbe>();
        let mut obj = ObjectValidation::default();
        obj.properties.insert("http".to_string(), http);
        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Object.into()),
            object: Some(Box::new(obj)),
            ..Default::default()
        })
    }
}

/// Build the combined JSON Schema for the two top-level YAML shapes:
/// a leaf Verification Definition (`verification:` / `criteria:`) or a
/// root manifest (`manifest_version:` / `verifications:`). The two are
/// discriminated structurally by the keys present.
///
/// Output is deterministic — schemars 0.8 backs its maps with a
/// `BTreeMap`, so key order is stable across runs.
pub fn json_schema() -> serde_json::Value {
    let mut generator = SchemaGenerator::default();
    // `subschema_for` adds each type (and its transitive deps) to the
    // generator's `definitions` and returns a `$ref` to it.
    let vd = generator.subschema_for::<VerificationDefinition>();
    let manifest = generator.subschema_for::<RootManifest>();

    let root = SchemaObject {
        metadata: Some(Box::new(Metadata {
            title: Some("Duhem Verification Definition / Root Manifest".to_string()),
            description: Some(
                "A leaf Verification Definition (has `verification:` / `criteria:`) or a root \
                 manifest (has `manifest_version:` / `verifications:`)."
                    .to_string(),
            ),
            ..Default::default()
        })),
        subschemas: Some(Box::new(SubschemaValidation {
            one_of: Some(vec![vd, manifest]),
            ..Default::default()
        })),
        ..Default::default()
    };

    let root_schema = RootSchema {
        meta_schema: generator.settings().meta_schema.clone(),
        definitions: generator.take_definitions(),
        schema: root,
    };

    serde_json::to_value(&root_schema).expect("schema serializes to JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_schema_has_expected_top_level_shape() {
        let schema = json_schema();
        let obj = schema.as_object().expect("schema is an object");

        // Emits a draft `$schema` header.
        assert!(
            obj.get("$schema").and_then(|v| v.as_str()).is_some(),
            "missing $schema header"
        );

        // Definitions for both top-level shapes are present.
        let defs = obj
            .get("definitions")
            .and_then(|v| v.as_object())
            .expect("definitions present");
        assert!(
            defs.contains_key("VerificationDefinition"),
            "definitions: {:?}",
            defs.keys().collect::<Vec<_>>()
        );
        assert!(
            defs.contains_key("RootManifest"),
            "definitions: {:?}",
            defs.keys().collect::<Vec<_>>()
        );

        // The VD definition mirrors `deny_unknown_fields`.
        let vd = defs
            .get("VerificationDefinition")
            .and_then(|v| v.as_object())
            .expect("VD def is an object");
        assert_eq!(
            vd.get("additionalProperties"),
            Some(&serde_json::Value::Bool(false)),
            "VD should set additionalProperties:false"
        );

        // Root is a oneOf of the two shapes.
        let one_of = obj
            .get("oneOf")
            .and_then(|v| v.as_array())
            .expect("root oneOf present");
        assert_eq!(one_of.len(), 2, "oneOf should union VD + manifest");
    }
}
