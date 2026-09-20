//! `duhem validate` must accept a whole-string `$` expression on a
//! closed-enum `with:` field, deferring the value check to runtime
//! exactly as spec §10.3 already specifies for every other `with:`
//! field (#533).
//!
//! `ui/assert-element`'s `expected:` is the only closed-enum `with:`
//! field in the v1 catalog, so it's the probe surface — but the fix
//! lives in the shared contract checker, not this one call site.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_duhem"))
}

fn validate(dir: &Path, expected_field: &str, declare_input: bool) -> Output {
    let inputs = if declare_input {
        "inputs:\n  expected_state: { type: string }\n"
    } else {
        ""
    };
    let yaml = format!(
        r#"verification: enum expression probe
{inputs}criteria:
  - id: AC-1
    description: d
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/assert-element
            with:
              locator: {{ css: h1 }}
              expected: {expected_field}
        assertions: ["true"]
"#
    );
    let path = dir.join("probe.yml");
    std::fs::write(&path, yaml).unwrap();
    Command::new(bin())
        .arg("validate")
        .arg(&path)
        .output()
        .expect("spawn duhem")
}

/// Literal control: an in-set literal value always validates. This is
/// what makes the probe result attributable to the `$` form rather
/// than to something else about the VD.
#[test]
fn literal_enum_value_is_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    let out = validate(tmp.path(), "visible", false);
    assert!(out.status.success(), "{:?}", String::from_utf8_lossy(&out.stderr));
}

/// Probe: a well-formed whole-string `$` expression on a closed-enum
/// field must validate, deferring the value check to runtime (§10.3).
#[test]
fn dollar_expression_on_enum_field_is_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    let out = validate(tmp.path(), "$inputs.expected_state", true);
    assert!(out.status.success(), "{:?}", String::from_utf8_lossy(&out.stderr));
}

/// A malformed expression is still rejected — proves the skip is
/// scoped to well-formed whole-string expressions, not to anything
/// starting with `$`. Caught by expression-syntax validation
/// (`duhem_schema::validate`), which runs ahead of the contract check.
#[test]
fn malformed_expression_on_enum_field_is_still_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let out = validate(tmp.path(), "$inputs.", true);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("$inputs."),
        "expected the malformed expression to be named in the error: {stderr}"
    );
}

/// A non-expression invalid literal is still rejected — proves the
/// enum check still discriminates and wasn't made vacuous.
#[test]
fn non_expression_invalid_literal_is_still_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let out = validate(tmp.path(), "vissible", false);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("vissible") && stderr.contains("is not valid"),
        "{stderr}"
    );
}
