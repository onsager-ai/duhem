//! Input resolution for `duhem run`: combine the merged `--inputs`
//! tokens (`KEY=VALUE` + `@file`, last-wins — see `inputs::merge_inputs`),
//! a selected named profile, a leaf- or manifest-declared process
//! `env:` fallback, and the applicable declaration's `default:` into
//! the engine's typed input map.
//!
//! Precedence, highest first (spec #68 / #151 / #346):
//!   --inputs  >  selected profile  >  process `env:`  >  default
//!
//! Lives in its own module so `main.rs` stays under the per-file token
//! budget.

use std::collections::BTreeMap;

use duhem_schema::{InputDecl, InputType, VerificationDefinition};

use crate::inputs::InputValue;

pub(crate) type ResolvedInputs = BTreeMap<String, serde_json::Value>;

/// Resolve one leaf and construct its masking registry from the same
/// effective declarations. Keeping these operations together prevents
/// a future precedence source from resolving a secret without also
/// registering it at the evidence boundary.
pub(crate) fn resolve_leaf_inputs(
    merged: &BTreeMap<String, InputValue>,
    profile: &BTreeMap<String, serde_json::Value>,
    definition: &VerificationDefinition,
    manifest_decls: &BTreeMap<String, InputDecl>,
) -> Result<(ResolvedInputs, duhem_evidence::SecretRegistry), String> {
    let values = resolve_inputs_with_manifest(merged, profile, &definition.inputs, manifest_decls)?;
    let mut secrets = secret_registry_with_manifest(&values, &definition.inputs, manifest_decls);
    // Flow bindings must be registered before `run_started` records the
    // composed definition snapshot. Runtime registration still covers
    // values acquired later, but it is too late for the run header.
    register_flow_secrets(&mut secrets, definition, Some(&values));
    Ok((values, secrets))
}

/// Resolve the merged `--inputs` map (spec #151) + an optional
/// selected-profile key map against the Verification Definition's
/// `inputs:` block. Precedence, highest first (spec #68 / #151):
///
/// 1. `--inputs` (the last-wins merge of `KEY=VALUE` + `@file` tokens):
///    a [`InputValue::Raw`] string is coerced per the declared
///    `InputType`; a [`InputValue::Typed`] value (from an `@file`) is
///    shape-validated against it.
/// 2. Selected profile's key `k` (spec #68) → validated against
///    the declared `InputType`. A profile key that matches no
///    declared input is *not* an error here (the profile may carry
///    keys that are only consumed via `$env.<key>`, not as inputs); it
///    simply doesn't feed input resolution.
/// 3. The process variable named by the leaf's `env:` declaration,
///    coerced with the same rules as a raw `--inputs` token.
/// 4. The VD's per-input `default:` (schema validator type-checked it
///    at parse time).
/// 5. None of the above leaves the input unbound. The runtime records
///    `Inconclusive(EnvironmentError)` before executing the workload.
///
/// Unknown inputs from `--inputs` remain hard errors (those name an
/// input explicitly); the profile map is consulted only for keys
/// that *are* declared inputs.
#[cfg(test)]
pub(crate) fn resolve_inputs(
    merged: &BTreeMap<String, InputValue>,
    profile: &BTreeMap<String, serde_json::Value>,
    decls: &BTreeMap<String, InputDecl>,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    resolve_inputs_with_manifest(merged, profile, decls, &BTreeMap::new())
}

/// Resolve inputs with suite-wide declarations available to inherited
/// names (spec #354). A declaration is consulted only when the leaf
/// marks that name `inherit: true`; an empty manifest map is exactly
/// the pre-#354 behavior.
pub(crate) fn resolve_inputs_with_manifest(
    merged: &BTreeMap<String, InputValue>,
    profile: &BTreeMap<String, serde_json::Value>,
    decls: &BTreeMap<String, InputDecl>,
    manifest_decls: &BTreeMap<String, InputDecl>,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    resolve_inputs_with_manifest_env(merged, profile, decls, manifest_decls, |name| {
        std::env::var(name).ok()
    })
}

/// Pure resolution seam used by tests to supply process-environment
/// values without mutating process-global state.
#[cfg(test)]
pub(crate) fn resolve_inputs_with_env(
    merged: &BTreeMap<String, InputValue>,
    profile: &BTreeMap<String, serde_json::Value>,
    decls: &BTreeMap<String, InputDecl>,
    process_env: impl Fn(&str) -> Option<String>,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    resolve_inputs_with_manifest_env(merged, profile, decls, &BTreeMap::new(), process_env)
}

/// Pure manifest-aware resolution seam used by tests to supply process
/// environment values without mutating process-global state.
pub(crate) fn resolve_inputs_with_manifest_env(
    merged: &BTreeMap<String, InputValue>,
    profile: &BTreeMap<String, serde_json::Value>,
    decls: &BTreeMap<String, InputDecl>,
    manifest_decls: &BTreeMap<String, InputDecl>,
    process_env: impl Fn(&str) -> Option<String>,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    for name in merged.keys() {
        if !decls.contains_key(name) {
            return Err(format!("unknown input: `{name}`"));
        }
    }
    let mut out = BTreeMap::new();
    for (name, decl) in decls.iter().filter(|(_, decl)| !decl.inherit) {
        if let Some(value) = resolve_decl(name, decl, merged, profile, &process_env)? {
            out.insert(name.clone(), value);
        }
    }
    // Inherited names use a manifest declaration when one exists
    // (spec #354), enforcing its type and full precedence chain. Without
    // one they retain the names-only #135 behavior byte-for-byte:
    // `--inputs` raw strings stay raw, selected profile values stay
    // unchecked, and there is no process/default layer.
    for (name, _) in decls.iter().filter(|(_, decl)| decl.inherit) {
        if let Some(decl) = manifest_decls.get(name) {
            if let Some(value) = resolve_decl(name, decl, merged, profile, &process_env)? {
                out.insert(name.clone(), value);
            }
        } else if let Some(value) = merged.get(name) {
            // An inherited name has no type to coerce to: a `KEY=VALUE`
            // token binds its raw string as-is; an `@file` value binds
            // its typed JSON.
            let v = match value {
                InputValue::Raw(raw_value) => serde_json::Value::String(raw_value.clone()),
                InputValue::Typed(typed) => typed.clone(),
            };
            out.insert(name.clone(), v);
        } else if let Some(profile_value) = profile.get(name) {
            out.insert(name.clone(), profile_value.clone());
        }
    }
    Ok(out)
}

fn resolve_decl(
    name: &str,
    decl: &InputDecl,
    merged: &BTreeMap<String, InputValue>,
    profile: &BTreeMap<String, serde_json::Value>,
    process_env: &impl Fn(&str) -> Option<String>,
) -> Result<Option<serde_json::Value>, String> {
    let kind = decl
        .kind
        .ok_or_else(|| format!("input `{name}`: missing `type:` declaration"))?;
    if let Some(value) = merged.get(name) {
        let resolved = match value {
            InputValue::Raw(raw_value) => coerce_input(name, kind, raw_value)?,
            InputValue::Typed(typed) => {
                validate_file_value(name, kind, typed)?;
                typed.clone()
            }
        };
        Ok(Some(resolved))
    } else if let Some(profile_value) = profile.get(name) {
        validate_profile_value(name, kind, profile_value)?;
        Ok(Some(profile_value.clone()))
    } else if let Some(env_name) = &decl.env
        && let Some(raw_value) = process_env(env_name)
    {
        coerce_input(name, kind, &raw_value).map(Some).map_err(|_| {
            format!(
                "input `{name}` (from env `{env_name}`): expected {}, got `{raw_value}`",
                kind
            )
        })
    } else if let Some(default) = &decl.default {
        yml_to_json(default)
            .map(Some)
            .map_err(|e| format!("input `{name}`: default: {e}"))
    } else {
        Ok(None)
    }
}

/// Build the leaf-only run registry retained for Pattern A callers and
/// regression tests.
#[cfg(test)]
pub(crate) fn secret_registry(
    values: &BTreeMap<String, serde_json::Value>,
    decls: &BTreeMap<String, InputDecl>,
) -> duhem_evidence::SecretRegistry {
    secret_registry_with_manifest(values, decls, &BTreeMap::new())
}

/// Build the run-scoped registry immediately after manifest-aware
/// resolution. Manifest secrets contribute only for leaves that
/// inherit their names; a same-named local input is never reclassified
/// by a suite declaration.
pub(crate) fn secret_registry_with_manifest(
    values: &BTreeMap<String, serde_json::Value>,
    decls: &BTreeMap<String, InputDecl>,
    manifest_decls: &BTreeMap<String, InputDecl>,
) -> duhem_evidence::SecretRegistry {
    let mut registry = duhem_evidence::SecretRegistry::new();
    for (name, decl) in decls {
        let manifest_secret = decl.inherit
            && manifest_decls
                .get(name)
                .is_some_and(|manifest| manifest.secret);
        if (decl.secret || manifest_secret)
            && let Some(value) = values.get(name)
        {
            registry.register_json(name.clone(), value);
        }
    }
    registry
}

/// Register secret flow-parameter bindings before the first evidence
/// event. Expanded steps retain the bindings in `flow_secrets`, including
/// structured values and references to resolved inputs.
pub(crate) fn register_flow_secrets(
    registry: &mut duhem_evidence::SecretRegistry,
    definition: &VerificationDefinition,
    resolved_inputs: Option<&ResolvedInputs>,
) {
    for step in definition
        .criteria
        .iter()
        .flat_map(|criterion| &criterion.checks)
        .flat_map(|check| &check.steps)
    {
        for (index, value) in step.flow_secrets.iter().enumerate() {
            let mut ordinal = 0usize;
            walk_flow_secret_leaves(value, &mut |leaf| {
                let name = if ordinal == 0 {
                    format!("flow_param_{index}")
                } else {
                    format!("flow_param_{index}_{ordinal}")
                };
                ordinal += 1;
                let resolved = leaf
                    .as_str()
                    .and_then(|raw| raw.strip_prefix("$inputs."))
                    .and_then(reference_head)
                    .and_then(|name| resolved_inputs.and_then(|inputs| inputs.get(name)))
                    .cloned()
                    .or_else(|| yml_to_json(leaf).ok());
                if let Some(value) = resolved {
                    registry.register_json(name, &value);
                }
            });
        }
    }
}

/// Visit every scalar leaf of a secret binding so structured flow
/// parameters receive the same exact-value masking as scalar bindings.
fn walk_flow_secret_leaves<F: FnMut(&serde_yml::Value)>(value: &serde_yml::Value, visit: &mut F) {
    match value {
        serde_yml::Value::Sequence(values) => {
            for value in values {
                walk_flow_secret_leaves(value, visit);
            }
        }
        serde_yml::Value::Mapping(values) => {
            for value in values.values() {
                walk_flow_secret_leaves(value, visit);
            }
        }
        leaf => visit(leaf),
    }
}

pub(crate) fn reference_head(reference: &str) -> Option<&str> {
    let end = reference.find(['.', '[', ')']).unwrap_or(reference.len());
    (end > 0).then_some(&reference[..end])
}

/// Type-check a value supplied by the selected profile against its
/// declared `InputType` — same shape rule as an `--inputs @file` value,
/// with an error string that points at the profile as the source.
fn validate_profile_value(
    name: &str,
    kind: InputType,
    v: &serde_json::Value,
) -> Result<(), String> {
    let actual = json_shape_name(v);
    let ok = match kind {
        InputType::String => matches!(v, serde_json::Value::String(_)),
        InputType::Integer => v.as_i64().is_some(),
        InputType::Number => v.is_number(),
        InputType::Boolean => matches!(v, serde_json::Value::Bool(_)),
        InputType::Array => matches!(v, serde_json::Value::Array(_)),
        InputType::Object => matches!(v, serde_json::Value::Object(_)),
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "input `{name}` (from profile): expected {kind}, got {actual}"
        ))
    }
}

/// Type-check a value loaded from an `--inputs @file` against its
/// declared `InputType`. The file's parser already gave us a typed
/// JSON value,
/// so this is a shape check, not a string coercion. Mirrors the
/// promotion rule used by the schema validator: an `integer` is a
/// valid `number`, but not vice versa.
fn validate_file_value(name: &str, kind: InputType, v: &serde_json::Value) -> Result<(), String> {
    let actual = json_shape_name(v);
    let ok = match kind {
        InputType::String => matches!(v, serde_json::Value::String(_)),
        InputType::Integer => v.as_i64().is_some(),
        InputType::Number => v.is_number(),
        InputType::Boolean => matches!(v, serde_json::Value::Bool(_)),
        InputType::Array => matches!(v, serde_json::Value::Array(_)),
        InputType::Object => matches!(v, serde_json::Value::Object(_)),
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "input `{name}` (from --inputs @file): expected {kind}, got {actual}"
        ))
    }
}

/// Coerce a `--inputs k=v` value to its declared `InputType`. Failure
/// surfaces as a CLI-friendly error naming the input and the expected
/// type.
fn coerce_input(name: &str, kind: InputType, v: &str) -> Result<serde_json::Value, String> {
    match kind {
        InputType::String => Ok(serde_json::Value::String(v.to_string())),
        InputType::Integer => v
            .parse::<i64>()
            .map(|n| serde_json::Value::Number(n.into()))
            .map_err(|_| format!("--inputs `{name}={v}`: expected integer, got `{v}`")),
        InputType::Number => {
            // Accept integer literals as `number`; serde_json picks the
            // narrowest representation. Fractional values stay
            // fractional.
            if let Ok(i) = v.parse::<i64>() {
                Ok(serde_json::Value::Number(i.into()))
            } else if let Ok(f) = v.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| {
                        format!("--inputs `{name}={v}`: number not representable as f64")
                    })
            } else {
                Err(format!("--inputs `{name}={v}`: expected number, got `{v}`"))
            }
        }
        InputType::Boolean => match v {
            // Strict per Alignment §"Boolean strictness at the CLI":
            // only the canonical `true` / `false` literals.
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            _ => Err(format!(
                "--inputs `{name}={v}`: expected boolean (`true` or `false`), got `{v}`"
            )),
        },
        InputType::Array => {
            let parsed: serde_json::Value = serde_json::from_str(v).map_err(|e| {
                format!("--inputs `{name}={v}`: expected JSON array, parse error: {e}")
            })?;
            if !parsed.is_array() {
                return Err(format!(
                    "--inputs `{name}={v}`: expected JSON array, got {}",
                    json_shape_name(&parsed)
                ));
            }
            Ok(parsed)
        }
        InputType::Object => {
            let parsed: serde_json::Value = serde_json::from_str(v).map_err(|e| {
                format!("--inputs `{name}={v}`: expected JSON object, parse error: {e}")
            })?;
            if !parsed.is_object() {
                return Err(format!(
                    "--inputs `{name}={v}`: expected JSON object, got {}",
                    json_shape_name(&parsed)
                ));
            }
            Ok(parsed)
        }
    }
}

/// Render a resolved input value for the `--dry-run` `RESOLVED INPUT`
/// block (spec #155). A string renders bare — no surrounding quotes —
/// so a black-box VD can assert the winning value directly off stdout;
/// every other JSON type renders as compact JSON, a deterministic and
/// parseable form for the *coerced* value (e.g. an `integer` input
/// shows `3`, a `boolean` shows `true`, an `object` shows `{"k":1}`).
pub(crate) fn render_input_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn json_shape_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Convert a YAML default value into JSON for engine consumption.
///
/// Fallible because YAML permits non-string mapping keys (e.g.
/// `default: { 1: "x" }`); JSON does not. Silently dropping such
/// entries would mutate the author's default; we surface them as a
/// user-facing error instead.
fn yml_to_json(v: &serde_yml::Value) -> Result<serde_json::Value, String> {
    use serde_yml::Value as Y;
    Ok(match v {
        Y::Null => serde_json::Value::Null,
        Y::Bool(b) => serde_json::Value::Bool(*b),
        Y::Number(n) => serde_json::to_value(n).unwrap_or(serde_json::Value::Null),
        Y::String(s) => serde_json::Value::String(s.clone()),
        Y::Sequence(seq) => {
            let mut out = Vec::with_capacity(seq.len());
            for item in seq {
                out.push(yml_to_json(item)?);
            }
            serde_json::Value::Array(out)
        }
        Y::Mapping(m) => {
            let mut out = serde_json::Map::with_capacity(m.len());
            for (k, v) in m {
                let key = k.as_str().ok_or_else(|| {
                    "object default has a non-string mapping key (not representable as JSON)"
                        .to_string()
                })?;
                out.insert(key.to_string(), yml_to_json(v)?);
            }
            serde_json::Value::Object(out)
        }
        Y::Tagged(t) => yml_to_json(&t.value)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use duhem_schema::VerificationDefinition;

    fn decls(yaml: &str) -> BTreeMap<String, InputDecl> {
        let src = format!("verification: x\ninputs:\n{yaml}\ncriteria: []\n");
        VerificationDefinition::from_yaml_str(&src)
            .expect("declarations parse")
            .inputs
    }

    #[test]
    fn manifest_declaration_enforces_type_only_when_present() {
        let leaf = decls("  count: { inherit: true }");
        let raw = BTreeMap::from([(
            "count".to_string(),
            InputValue::Raw("not-an-integer".to_string()),
        )]);
        let declared = decls("  count: { type: integer }");

        let err =
            resolve_inputs_with_manifest_env(&raw, &BTreeMap::new(), &leaf, &declared, |_| None)
                .unwrap_err();
        assert!(
            err.contains("count") && err.contains("integer"),
            "manifest declaration owns the diagnostic: {err}"
        );

        let legacy = resolve_inputs_with_manifest_env(
            &raw,
            &BTreeMap::new(),
            &leaf,
            &BTreeMap::new(),
            |_| None,
        )
        .unwrap();
        assert_eq!(
            legacy["count"],
            serde_json::json!("not-an-integer"),
            "an undeclared inherited name keeps its pre-#354 raw-string behavior"
        );
    }

    #[test]
    fn manifest_declaration_uses_documented_precedence() {
        let leaf = decls("  count: { inherit: true }");
        let declared = decls("  count: { type: integer, env: APP_COUNT, default: 1 }");
        let selected = BTreeMap::from([("count".to_string(), serde_json::json!(2))]);
        let process = |name: &str| (name == "APP_COUNT").then(|| "3".to_string());

        let from_default = resolve_inputs_with_manifest_env(
            &BTreeMap::new(),
            &BTreeMap::new(),
            &leaf,
            &declared,
            |_| None,
        )
        .unwrap();
        assert_eq!(from_default["count"], serde_json::json!(1));

        let from_process = resolve_inputs_with_manifest_env(
            &BTreeMap::new(),
            &BTreeMap::new(),
            &leaf,
            &declared,
            process,
        )
        .unwrap();
        assert_eq!(from_process["count"], serde_json::json!(3));

        let from_selected = resolve_inputs_with_manifest_env(
            &BTreeMap::new(),
            &selected,
            &leaf,
            &declared,
            process,
        )
        .unwrap();
        assert_eq!(from_selected["count"], serde_json::json!(2));

        let explicit = BTreeMap::from([("count".to_string(), InputValue::Raw("4".to_string()))]);
        let from_flag =
            resolve_inputs_with_manifest_env(&explicit, &selected, &leaf, &declared, process)
                .unwrap();
        assert_eq!(from_flag["count"], serde_json::json!(4));
    }

    #[test]
    fn manifest_secret_is_registered_for_each_inheriting_leaf() {
        let leaf = decls("  password: { inherit: true }");
        let declared = decls("  password: { type: string, secret: true }");
        let values = BTreeMap::from([(
            "password".to_string(),
            serde_json::json!("high-entropy-value"),
        )]);

        for _leaf in 0..2 {
            let registry = secret_registry_with_manifest(&values, &leaf, &declared);
            assert_eq!(
                registry.mask("high-entropy-value").text,
                "[redacted:password]"
            );
        }
    }

    #[test]
    fn leaf_can_add_secret_protection_to_inherited_input() {
        let leaf = decls("  password: { inherit: true, secret: true }");
        let declared = decls("  password: { type: string }");
        let values = BTreeMap::from([(
            "password".to_string(),
            serde_json::json!("high-entropy-value"),
        )]);

        let registry = secret_registry_with_manifest(&values, &leaf, &declared);
        assert_eq!(
            registry.mask("high-entropy-value").text,
            "[redacted:password]"
        );
    }
}
