//! Input resolution for `duhem run`: combine the merged `--inputs`
//! tokens (`KEY=VALUE` + `@file`, last-wins — see `inputs::merge_inputs`),
//! a selected named environment, and the VD's per-input `default:` into
//! the engine's typed input map.
//!
//! Precedence, highest first (spec #68 / #151):
//!   --inputs (last-wins merge)  >  selected environment  >  default
//!
//! Lives in its own module so `main.rs` stays under the per-file token
//! budget.

use std::collections::BTreeMap;

use duhem_schema::{InputDecl, InputType};

use crate::inputs::InputValue;

/// Resolve the merged `--inputs` map (spec #151) + an optional
/// selected-environment key map against the Verification Definition's
/// `inputs:` block. Precedence, highest first (spec #68 / #151):
///
/// 1. `--inputs` (the last-wins merge of `KEY=VALUE` + `@file` tokens):
///    a [`InputValue::Raw`] string is coerced per the declared
///    `InputType`; a [`InputValue::Typed`] value (from an `@file`) is
///    shape-validated against it.
/// 2. Selected environment's key `k` (spec #68) → validated against
///    the declared `InputType`. An environment key that matches no
///    declared input is *not* an error here (the environment may carry
///    keys that are only consumed via `$env.<key>`, not as inputs); it
///    simply doesn't feed input resolution.
/// 3. The VD's per-input `default:` (schema validator type-checked it
///    at parse time).
/// 4. None of the above + no default → error.
///
/// Unknown inputs from `--inputs` remain hard errors (those name an
/// input explicitly); the environment map is consulted only for keys
/// that *are* declared inputs.
pub(crate) fn resolve_inputs(
    merged: &BTreeMap<String, InputValue>,
    env: &BTreeMap<String, serde_json::Value>,
    decls: &BTreeMap<String, InputDecl>,
    inherits: &[String],
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    // An `--inputs` key is "known" if it names a declared input *or* an
    // inherited name (spec #135) — an inherited name has no local
    // `InputDecl`, but the leaf still accepts a value for it on the
    // precedence chain.
    let is_known = |name: &str| decls.contains_key(name) || inherits.iter().any(|n| n == name);
    for name in merged.keys() {
        if !is_known(name) {
            return Err(format!("unknown input: `{name}`"));
        }
    }
    let mut out = BTreeMap::new();
    for (name, decl) in decls {
        if let Some(value) = merged.get(name) {
            let resolved = match value {
                InputValue::Raw(raw_value) => coerce_input(name, decl.kind, raw_value)?,
                InputValue::Typed(typed) => {
                    validate_file_value(name, decl.kind, typed)?;
                    typed.clone()
                }
            };
            out.insert(name.clone(), resolved);
        } else if let Some(env_value) = env.get(name) {
            validate_env_value(name, decl.kind, env_value)?;
            out.insert(name.clone(), env_value.clone());
        } else if let Some(default) = &decl.default {
            let value =
                yml_to_json(default).map_err(|e| format!("input `{name}`: default: {e}"))?;
            out.insert(name.clone(), value);
        } else {
            return Err(format!("missing required input: `{name}`"));
        }
    }
    // Inherited names (spec #135): no local `InputDecl`, so no
    // `InputType` to coerce/validate against — bind the value from the
    // precedence chain (`--inputs` > selected environment), skipping any
    // local `default:` layer (there is none). A name that resolves to
    // nothing is left UNBOUND here; it is not an error at resolution
    // time — a referenced-but-unbound inherited input fails loudly at
    // run time with the suite/--inputs remedy (the runtime's
    // `UnresolvedInheritedInput`). A name already bound as a declared
    // input (the `inputs ∩ inherits` overlap that the validator rejects)
    // is not overwritten.
    for name in inherits {
        if out.contains_key(name) {
            continue;
        }
        if let Some(value) = merged.get(name) {
            // An inherited name has no type to coerce to: a `KEY=VALUE`
            // token binds its raw string as-is; an `@file` value binds
            // its typed JSON.
            let v = match value {
                InputValue::Raw(raw_value) => serde_json::Value::String(raw_value.clone()),
                InputValue::Typed(typed) => typed.clone(),
            };
            out.insert(name.clone(), v);
        } else if let Some(env_value) = env.get(name) {
            out.insert(name.clone(), env_value.clone());
        }
    }
    Ok(out)
}

/// Type-check a value supplied by the selected environment against its
/// declared `InputType` — same shape rule as an `--inputs @file` value,
/// with an error string that points at the environment as the source.
fn validate_env_value(name: &str, kind: InputType, v: &serde_json::Value) -> Result<(), String> {
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
            "input `{name}` (from environment): expected {kind}, got {actual}"
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
