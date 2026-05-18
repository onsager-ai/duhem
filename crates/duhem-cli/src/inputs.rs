//! `--inputs-file` loader (spec on issue #33).
//!
//! Loads a YAML or JSON file containing a top-level `key: value`
//! mapping and returns it as a typed `BTreeMap<String, serde_json::Value>`.
//! Selection is by extension: `.yml` / `.yaml` parse as YAML, `.json`
//! parses as JSON. The two share an in-memory representation
//! (`serde_yml::Value` → `serde_json::Value`) so downstream resolution
//! is identical regardless of source format.
//!
//! Conflict resolution between this file and `--inputs k=v` is the
//! caller's responsibility (see `main::resolve_inputs`); per #33
//! Alignment, explicit `--inputs` always wins on the same key.

use std::collections::BTreeMap;
use std::path::Path;

/// Read `path`, parse it as YAML or JSON (by extension), and return the
/// top-level mapping as a `name -> json` map. Errors carry the file
/// path so a missing or malformed inputs file is easy to locate.
pub fn load_inputs_file(path: &Path) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| format!("--inputs-file {}: {e}", path.display()))?;

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
        Some("json") => serde_json::from_str(&src)
            .map_err(|e| format!("--inputs-file {}: {e}", path.display()))?,
        Some("yml") | Some("yaml") | None => {
            let yml: serde_yml::Value = serde_yml::from_str(&src)
                .map_err(|e| format!("--inputs-file {}: {e}", path.display()))?;
            yml_to_json(&yml).map_err(|e| format!("--inputs-file {}: {e}", path.display()))?
        }
        Some(other) => {
            return Err(format!(
                "--inputs-file {}: unsupported extension `.{other}` (expected .yml, .yaml, or .json)",
                path.display()
            ));
        }
    };

    let map = match value {
        serde_json::Value::Object(m) => m,
        // Reject empty / non-mapping top-level documents explicitly:
        // silently treating `null` or a bare scalar as "no inputs"
        // would mask an authoring error in the file the user just
        // pointed `--inputs-file` at.
        serde_json::Value::Null => {
            return Err(format!(
                "--inputs-file {}: file is empty or contains only null; expected a key/value mapping",
                path.display()
            ));
        }
        other => {
            return Err(format!(
                "--inputs-file {}: expected a key/value mapping at the top level, got {}",
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
}
