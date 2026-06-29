//! `--inputs` token parsing (spec on issues #33 / #151).
//!
//! `duhem run --inputs` accepts two token shapes, repeatable and
//! mixable in one invocation:
//!
//! - `KEY=VALUE` — a single input, the value coerced later per the
//!   declared `InputType` (a *raw* string here).
//! - `@PATH` — a YAML or JSON file containing a top-level `key: value`
//!   mapping; every key contributes a *typed* JSON value. Selection is
//!   by extension: `.yml` / `.yaml` parse as YAML, `.json` parses as
//!   JSON. The two share an in-memory representation
//!   (`serde_yml::Value` → `serde_json::Value`) so downstream
//!   resolution is identical regardless of source format.
//!
//! Tokens are processed left-to-right and **last occurrence wins** on a
//! given key (spec #151): `--inputs @base.yml --inputs k=v --inputs
//! @override.yml` resolves `k` from whichever token mentioned it last.
//! A whole token starting with `@` and containing no `=` is a file
//! ref; `key=@literal` keeps `@literal` as a *literal* value (the `@`
//! only triggers file-loading as a bare leading token, never after
//! `=`). The merged map (see [`merge_inputs`]) is precedence layer 1 in
//! `resolve::resolve_inputs`, above the selected environment and the
//! VD `default:`.

use std::collections::BTreeMap;
use std::path::Path;

/// One resolved `--inputs` value, tagged by provenance so resolution
/// applies the right rule: a `KEY=VALUE` token is a [`InputValue::Raw`]
/// string coerced per the declared `InputType`; a value sourced from an
/// `@file` is an already-typed [`InputValue::Typed`] JSON value, shape-
/// checked rather than coerced.
#[derive(Debug, Clone)]
pub(crate) enum InputValue {
    Raw(String),
    Typed(serde_json::Value),
}

/// Fold the ordered `--inputs` tokens into a last-wins map. Each token
/// is either a bare `@PATH` file ref (loaded via [`load_inputs_file`],
/// each key contributing a [`InputValue::Typed`]) or a `KEY=VALUE` pair
/// (a [`InputValue::Raw`] string). A later token mentioning a key
/// replaces an earlier one (spec #151). `@PATH` loading happens here,
/// before any browser launch, so a missing/malformed `@file` fails
/// fast.
pub(crate) fn merge_inputs(tokens: &[String]) -> Result<BTreeMap<String, InputValue>, String> {
    let mut out: BTreeMap<String, InputValue> = BTreeMap::new();
    for tok in tokens {
        // A bare leading `@` with no `=` is a file ref. Anything with an
        // `=` (including `key=@literal`) is a `KEY=VALUE` pair — the `@`
        // never triggers file-loading after the `=`.
        if let Some(path) = tok.strip_prefix('@')
            && !tok.contains('=')
        {
            let map = load_inputs_file(Path::new(path))?;
            for (k, v) in map {
                out.insert(k, InputValue::Typed(v));
            }
            continue;
        }
        let (k, v) = tok
            .split_once('=')
            .ok_or_else(|| format!("--inputs `{tok}`: expected `key=value` or `@file`"))?;
        if k.is_empty() {
            return Err(format!("--inputs `{tok}`: empty key"));
        }
        out.insert(k.to_string(), InputValue::Raw(v.to_string()));
    }
    Ok(out)
}

/// Read `path`, parse it as YAML or JSON (by extension), and return the
/// top-level mapping as a `name -> json` map. Errors carry the file
/// path so a missing or malformed inputs file is easy to locate.
pub fn load_inputs_file(path: &Path) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let src =
        std::fs::read_to_string(path).map_err(|e| format!("--inputs @{}: {e}", path.display()))?;

    // Extension-driven parser selection. YAML is a JSON superset in
    // practice (serde_yml accepts JSON), but keeping the parsers
    // separate gives JSON-only callers a cleaner error message when
    // their file is malformed.
    let value: serde_json::Value = match path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => {
            serde_json::from_str(&src).map_err(|e| format!("--inputs @{}: {e}", path.display()))?
        }
        Some("yml") | Some("yaml") | None => {
            let yml: serde_yml::Value = serde_yml::from_str(&src)
                .map_err(|e| format!("--inputs @{}: {e}", path.display()))?;
            yml_to_json(&yml).map_err(|e| format!("--inputs @{}: {e}", path.display()))?
        }
        Some(other) => {
            return Err(format!(
                "--inputs @{}: unsupported extension `.{other}` (expected .yml, .yaml, or .json)",
                path.display()
            ));
        }
    };

    let map = match value {
        serde_json::Value::Object(m) => m,
        // Reject empty / non-mapping top-level documents explicitly:
        // silently treating `null` or a bare scalar as "no inputs"
        // would mask an authoring error in the file the user just
        // pointed `--inputs @file` at.
        serde_json::Value::Null => {
            return Err(format!(
                "--inputs @{}: file is empty or contains only null; expected a key/value mapping",
                path.display()
            ));
        }
        other => {
            return Err(format!(
                "--inputs @{}: expected a key/value mapping at the top level, got {}",
                path.display(),
                json_shape_name(&other)
            ));
        }
    };

    Ok(map.into_iter().collect())
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

/// Convert a YAML value into the equivalent JSON value. Non-string
/// mapping keys are rejected explicitly — JSON requires string keys
/// and silently dropping such entries would mutate the author's
/// inputs file.
pub(crate) fn yml_to_json(v: &serde_yml::Value) -> Result<serde_json::Value, String> {
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
                    "object has a non-string mapping key (not representable as JSON)".to_string()
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
    use std::io::Write;

    fn write_tmp(ext: &str, body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("inputs.{ext}"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        (dir, path)
    }

    #[test]
    fn loads_yaml_with_mixed_types() {
        let (_d, p) = write_tmp(
            "yml",
            "base_url: http://localhost:3000\ncount: 3\nallow: true\n",
        );
        let m = load_inputs_file(&p).unwrap();
        assert_eq!(m["base_url"], serde_json::json!("http://localhost:3000"));
        assert_eq!(m["count"], serde_json::json!(3));
        assert_eq!(m["allow"], serde_json::json!(true));
    }

    #[test]
    fn loads_json_with_mixed_types() {
        let (_d, p) = write_tmp(
            "json",
            r#"{"base_url":"http://x","count":7,"flags":{"dark":true}}"#,
        );
        let m = load_inputs_file(&p).unwrap();
        assert_eq!(m["base_url"], serde_json::json!("http://x"));
        assert_eq!(m["count"], serde_json::json!(7));
        assert_eq!(m["flags"], serde_json::json!({"dark": true}));
    }

    #[test]
    fn missing_file_reports_path() {
        let err = load_inputs_file(std::path::Path::new("/no/such/file.yml")).unwrap_err();
        assert!(
            err.contains("/no/such/file.yml"),
            "error should name the path: {err}"
        );
    }

    #[test]
    fn top_level_non_mapping_is_rejected() {
        let (_d, p) = write_tmp("yml", "- one\n- two\n");
        let err = load_inputs_file(&p).unwrap_err();
        assert!(err.contains("mapping"), "got: {err}");
    }

    #[test]
    fn unsupported_extension_is_rejected() {
        let (_d, p) = write_tmp("toml", "x = 1\n");
        let err = load_inputs_file(&p).unwrap_err();
        assert!(err.contains("unsupported extension"), "got: {err}");
    }

    // ---- #151: `--inputs` token parser (mix, last-wins, literals) ----

    #[test]
    fn merge_parses_key_value_as_raw() {
        let m = merge_inputs(&["a=1".into(), "b=hello world".into()]).unwrap();
        assert!(matches!(&m["a"], InputValue::Raw(s) if s == "1"));
        assert!(matches!(&m["b"], InputValue::Raw(s) if s == "hello world"));
    }

    #[test]
    fn merge_loads_bare_at_file_as_typed() {
        let (_d, p) = write_tmp("yml", "base_url: http://x\ncount: 3\n");
        let m = merge_inputs(&[format!("@{}", p.display())]).unwrap();
        assert!(
            matches!(&m["base_url"], InputValue::Typed(v) if v == &serde_json::json!("http://x"))
        );
        assert!(matches!(&m["count"], InputValue::Typed(v) if v == &serde_json::json!(3)));
    }

    #[test]
    fn merge_last_token_wins_across_mixed_tokens() {
        let (_da, a) = write_tmp("yml", "k: from-a\n");
        let (_db, b) = write_tmp("yml", "k: from-b\n");
        // `@a` then `k=flag` then `@b` → the last mention (`@b`) wins.
        let m = merge_inputs(&[
            format!("@{}", a.display()),
            "k=from-flag".into(),
            format!("@{}", b.display()),
        ])
        .unwrap();
        assert!(matches!(&m["k"], InputValue::Typed(v) if v == &serde_json::json!("from-b")));
    }

    #[test]
    fn merge_flag_after_file_wins() {
        let (_d, a) = write_tmp("yml", "k: from-file\n");
        let m = merge_inputs(&[format!("@{}", a.display()), "k=from-flag".into()]).unwrap();
        assert!(matches!(&m["k"], InputValue::Raw(s) if s == "from-flag"));
    }

    #[test]
    fn key_at_literal_stays_literal_value() {
        // The `@` only triggers file-loading as a bare leading token; an
        // `@` after `=` is part of the literal value, never a file ref.
        let m = merge_inputs(&["name=@notafile.yml".into()]).unwrap();
        assert!(matches!(&m["name"], InputValue::Raw(s) if s == "@notafile.yml"));
    }

    #[test]
    fn bare_at_missing_file_errors_with_path() {
        let err = merge_inputs(&["@/no/such/inputs.yml".into()]).unwrap_err();
        assert!(err.contains("/no/such/inputs.yml"), "got: {err}");
    }

    #[test]
    fn token_without_equals_or_at_errors() {
        let err = merge_inputs(&["lonely".into()]).unwrap_err();
        assert!(
            err.contains("key=value") || err.contains("@file"),
            "got: {err}"
        );
    }

    #[test]
    fn empty_key_errors() {
        let err = merge_inputs(&["=v".into()]).unwrap_err();
        assert!(err.contains("empty key"), "got: {err}");
    }
}
