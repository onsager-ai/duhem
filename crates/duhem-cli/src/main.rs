//! `duhem` — the command-line entry point.
//!
//! Phase-0 skeleton. The free CLI binary (`docs/duhem-spec.md` §13)
//! ultimately offers `init`, `validate`, and `run`; subcommands land
//! in `spec(cli): duhem init / validate / run skeletons`. `validate`
//! and the minimal `run` form (`<file> [--inputs k=v ...]`) ship
//! today; the larger CLI surface (`--filter`, `--headed`,
//! `--evidence-dir`, `--reporter`) lands with `spec(cli): duhem run`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use duhem_actions::RunBrowser;
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::{InputDecl, InputType, VerificationDefinition, validate};

/// Duhem — holistic verification for AI-delivered software.
#[derive(Debug, Parser)]
#[command(name = "duhem", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Parse and structurally validate a Verification Definition file.
    Validate {
        /// Path to a `.yml` Verification Definition.
        path: PathBuf,
    },
    /// Execute a Verification Definition end-to-end.
    ///
    /// Minimal v1 form per `spec(runtime): minimal step executor`
    /// (issue #15). Strings-only `--inputs`; richer flags
    /// (`--filter`, `--headed`, `--evidence-dir`, `--reporter`) land
    /// with the full `spec(cli): duhem run`.
    Run {
        /// Path to a `.yml` Verification Definition.
        path: PathBuf,
        /// `key=value` inputs, repeatable.
        #[arg(long = "inputs", value_name = "KEY=VALUE")]
        inputs: Vec<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        None => ExitCode::SUCCESS,
        Some(Cmd::Validate { path }) => match run_validate(&path) {
            Ok(()) => {
                println!("OK");
                ExitCode::SUCCESS
            }
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        },
        Some(Cmd::Run { path, inputs }) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(run_command(path, inputs))
        }
    }
}

fn run_validate(path: &std::path::Path) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let v = VerificationDefinition::from_yaml_str(&src).map_err(|e| match e.location() {
        Some(loc) => format!("{}:{}:{}: {e}", path.display(), loc.line(), loc.column()),
        None => format!("{}: {e}", path.display()),
    })?;
    validate(&v).map_err(|errs| {
        let plural = if errs.len() == 1 { "" } else { "s" };
        let mut s = format!("{} validation error{plural}:", errs.len());
        for e in errs {
            s.push_str("\n  - ");
            s.push_str(&e.to_string());
        }
        s
    })
}

async fn run_command(path: PathBuf, raw_inputs: Vec<String>) -> ExitCode {
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let def = match VerificationDefinition::from_yaml_str(&src) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    // CLI-side fail-fast per the typed-input-catalog spec: missing /
    // unknown / mistyped inputs error before `Engine::run` is called.
    let inputs = match resolve_inputs(&raw_inputs, &def.inputs) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let browser = match RunBrowser::launch(false).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("browser: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut engine = Engine::new()
        .with_browser(browser)
        .with_definition_path(path.display().to_string());
    let verdict = match engine.run(&def, inputs).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("engine: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("{}", verdict.state);
    match verdict.state {
        VerdictState::Pass => ExitCode::SUCCESS,
        // Both Fail and Inconclusive must gate downstream actions —
        // exit non-zero so a CI step that ignores stdout still
        // notices.
        _ => ExitCode::FAILURE,
    }
}

/// Parse the raw `--inputs k=v` flags into a `(name, raw)` map.
fn parse_inputs(raw: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for s in raw {
        let (k, v) = s
            .split_once('=')
            .ok_or_else(|| format!("--inputs `{s}`: expected `key=value`"))?;
        if k.is_empty() {
            return Err(format!("--inputs `{s}`: empty key"));
        }
        out.insert(k.to_string(), v.to_string());
    }
    Ok(out)
}

/// Resolve `--inputs k=v` flags against the Verification Definition's
/// `inputs:` block. Per the typed-input-catalog spec:
///
/// - Unknown input → error.
/// - Provided value → coerced per declared `InputType`.
/// - Not provided + default present → default carried through as-is
///   (the schema validator already type-checked it).
/// - Not provided + no default → error.
fn resolve_inputs(
    raw: &[String],
    decls: &BTreeMap<String, InputDecl>,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let provided = parse_inputs(raw)?;
    for name in provided.keys() {
        if !decls.contains_key(name) {
            return Err(format!("unknown input: `{name}`"));
        }
    }
    let mut out = BTreeMap::new();
    for (name, decl) in decls {
        if let Some(raw_value) = provided.get(name) {
            let coerced = coerce_input(name, decl.kind, raw_value)?;
            out.insert(name.clone(), coerced);
        } else if let Some(default) = &decl.default {
            out.insert(name.clone(), yml_to_json(default));
        } else {
            return Err(format!("missing required input: `{name}`"));
        }
    }
    Ok(out)
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

fn yml_to_json(v: &serde_yml::Value) -> serde_json::Value {
    use serde_yml::Value as Y;
    match v {
        Y::Null => serde_json::Value::Null,
        Y::Bool(b) => serde_json::Value::Bool(*b),
        Y::Number(n) => serde_json::to_value(n).unwrap_or(serde_json::Value::Null),
        Y::String(s) => serde_json::Value::String(s.clone()),
        Y::Sequence(seq) => serde_json::Value::Array(seq.iter().map(yml_to_json).collect()),
        Y::Mapping(m) => serde_json::Value::Object(
            m.iter()
                .filter_map(|(k, v)| {
                    let key = k.as_str()?.to_string();
                    Some((key, yml_to_json(v)))
                })
                .collect(),
        ),
        Y::Tagged(t) => yml_to_json(&t.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decls(yaml: &str) -> BTreeMap<String, InputDecl> {
        let y = format!("verification: x\ninputs:\n{yaml}\ncriteria: []\n");
        VerificationDefinition::from_yaml_str(&y)
            .expect("parse")
            .inputs
    }

    fn raw(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn coerces_integer_input() {
        let d = decls("  count: { type: integer }");
        let out = resolve_inputs(&raw(&["count=3"]), &d).expect("ok");
        assert_eq!(out["count"], serde_json::json!(3));
    }

    #[test]
    fn integer_rejects_non_numeric() {
        let d = decls("  count: { type: integer }");
        let err = resolve_inputs(&raw(&["count=foo"]), &d).unwrap_err();
        assert!(err.contains("count"), "error names the input: {err}");
        assert!(
            err.contains("integer"),
            "error names the expected type: {err}"
        );
    }

    #[test]
    fn integer_rejects_fractional() {
        let d = decls("  count: { type: integer }");
        let err = resolve_inputs(&raw(&["count=1.5"]), &d).unwrap_err();
        assert!(err.contains("count"), "error names the input: {err}");
    }

    #[test]
    fn number_accepts_fractional_and_integer() {
        let d = decls("  threshold: { type: number }");
        let frac = resolve_inputs(&raw(&["threshold=0.85"]), &d).unwrap();
        assert_eq!(frac["threshold"], serde_json::json!(0.85));
        let whole = resolve_inputs(&raw(&["threshold=1"]), &d).unwrap();
        assert_eq!(whole["threshold"], serde_json::json!(1));
    }

    #[test]
    fn boolean_accepts_only_true_or_false() {
        let d = decls("  flag: { type: boolean }");
        let t = resolve_inputs(&raw(&["flag=true"]), &d).unwrap();
        assert_eq!(t["flag"], serde_json::json!(true));
        let f = resolve_inputs(&raw(&["flag=false"]), &d).unwrap();
        assert_eq!(f["flag"], serde_json::json!(false));
        // `1` / `yes` are rejected per the Alignment §"Boolean
        // strictness" decision: shell ergonomics don't justify
        // ambiguous parses for a verifier.
        for bad in ["1", "0", "yes", "no", "True", "FALSE"] {
            let err = resolve_inputs(&raw(&[&format!("flag={bad}")]), &d).unwrap_err();
            assert!(err.contains("boolean"), "rejecting `{bad}`: {err}");
        }
    }

    #[test]
    fn string_takes_value_literally() {
        // String values are NOT JSON-parsed — `--inputs name=foo`
        // gives the literal `foo`, never the JSON parse of `foo`
        // (which would error).
        let d = decls("  name: { type: string }");
        let out = resolve_inputs(&raw(&["name=hello world"]), &d).unwrap();
        assert_eq!(out["name"], serde_json::json!("hello world"));
    }

    #[test]
    fn array_parses_as_json() {
        let d = decls("  roles: { type: array }");
        let out = resolve_inputs(&raw(&[r#"roles=["admin","viewer"]"#]), &d).unwrap();
        assert_eq!(out["roles"], serde_json::json!(["admin", "viewer"]));
    }

    #[test]
    fn array_rejects_object_json() {
        let d = decls("  roles: { type: array }");
        let err = resolve_inputs(&raw(&[r#"roles={"a":1}"#]), &d).unwrap_err();
        assert!(err.contains("array"), "error names expected type: {err}");
    }

    #[test]
    fn object_parses_as_json() {
        let d = decls("  flags: { type: object }");
        let out = resolve_inputs(&raw(&[r#"flags={"dark":true}"#]), &d).unwrap();
        assert_eq!(out["flags"], serde_json::json!({"dark": true}));
    }

    #[test]
    fn object_rejects_array_json() {
        let d = decls("  flags: { type: object }");
        let err = resolve_inputs(&raw(&[r#"flags=[1,2]"#]), &d).unwrap_err();
        assert!(err.contains("object"), "error names expected type: {err}");
    }

    #[test]
    fn missing_required_input_errors() {
        let d = decls("  count: { type: integer }");
        let err = resolve_inputs(&raw(&[]), &d).unwrap_err();
        assert!(
            err.contains("missing required input") && err.contains("count"),
            "got: {err}"
        );
    }

    #[test]
    fn unknown_input_errors() {
        let d = decls("  count: { type: integer }");
        let err = resolve_inputs(&raw(&["count=1", "bogus=3"]), &d).unwrap_err();
        assert!(
            err.contains("unknown input") && err.contains("bogus"),
            "got: {err}"
        );
    }

    #[test]
    fn declared_default_is_used_when_input_absent() {
        let d = decls("  name: { type: string, default: \"ws-default\" }");
        let out = resolve_inputs(&raw(&[]), &d).unwrap();
        assert_eq!(out["name"], serde_json::json!("ws-default"));
    }

    #[test]
    fn explicit_input_overrides_default() {
        let d = decls("  name: { type: string, default: \"ws-default\" }");
        let out = resolve_inputs(&raw(&["name=other"]), &d).unwrap();
        assert_eq!(out["name"], serde_json::json!("other"));
    }
}
