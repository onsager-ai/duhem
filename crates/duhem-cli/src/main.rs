//! `duhem` — the command-line entry point.
//!
//! Phase-0 skeleton. The free CLI binary (`docs/duhem-spec.md` §13)
//! ultimately offers `init`, `validate`, and `run`; the `run`
//! subcommand carries the full authoring-ergonomics surface
//! (`--filter`, `--headed`, `--evidence-dir`, `--reporter`) per the
//! spec on issue #23. None of those flags are correctness gates —
//! they make iteration on Verification Definitions practical.

mod filter;
mod reporter;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use duhem_actions::RunBrowser;
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::{InputDecl, InputType, VerificationDefinition, validate};

use crate::filter::CliCheckFilter;
use crate::reporter::Reporter;

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
    /// `--filter`, `--headed`, `--evidence-dir`, `--reporter` are
    /// authoring-ergonomics flags from the spec on issue #23; none
    /// change the verdict on a non-filtered run.
    Run {
        /// Path to a `.yml` Verification Definition.
        path: PathBuf,
        /// `key=value` inputs, repeatable.
        #[arg(long = "inputs", value_name = "KEY=VALUE")]
        inputs: Vec<String>,
        /// Limit the run to a subset of `(criterion, check)` pairs.
        ///
        /// Grammar: `AC-1` (every check under `AC-1`),
        /// `AC-1::AC-1.2` (one pair), `AC-*::AC-*.1` (globbed). Repeat
        /// the flag to OR patterns: `--filter AC-1 --filter AC-2`.
        #[arg(long = "filter", value_name = "PATTERN")]
        filter: Vec<String>,
        /// Launch the browser with a visible window. Default is
        /// headless. Has no effect on `api/*` actions.
        #[arg(long = "headed", default_value_t = false)]
        headed: bool,
        /// Directory under which `EvidenceWriter` creates the per-run
        /// trace. Falls back to the engine default (`.duhem/runs`)
        /// when absent; created if missing.
        #[arg(long = "evidence-dir", value_name = "PATH")]
        evidence_dir: Option<PathBuf>,
        /// Stdout formatting for the post-run summary:
        /// `default` (verdict line), `quiet` (exit code only),
        /// `json` (one-line summary `{run_id, verdict, criteria,
        /// evidence_dir}`).
        #[arg(long = "reporter", value_enum, default_value_t = ReporterArg::Default)]
        reporter: ReporterArg,
    },
}

/// `clap`-facing reporter enum, kept separate from `reporter::Reporter`
/// so the reporter module stays CLI-dep-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ReporterArg {
    Default,
    Quiet,
    Json,
}

impl From<ReporterArg> for Reporter {
    fn from(r: ReporterArg) -> Self {
        match r {
            ReporterArg::Default => Reporter::Default,
            ReporterArg::Quiet => Reporter::Quiet,
            ReporterArg::Json => Reporter::Json,
        }
    }
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
        Some(Cmd::Run {
            path,
            inputs,
            filter,
            headed,
            evidence_dir,
            reporter,
        }) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(run_command(RunArgs {
                path,
                inputs,
                filter,
                headed,
                evidence_dir,
                reporter: reporter.into(),
            }))
        }
    }
}

/// Resolved `duhem run` arguments. Kept as a struct so the dispatch
/// function's signature doesn't grow unbounded as new flags land.
struct RunArgs {
    path: PathBuf,
    inputs: Vec<String>,
    filter: Vec<String>,
    headed: bool,
    evidence_dir: Option<PathBuf>,
    reporter: Reporter,
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

async fn run_command(args: RunArgs) -> ExitCode {
    let RunArgs {
        path,
        inputs: raw_inputs,
        filter: raw_filter,
        headed,
        evidence_dir,
        reporter,
    } = args;

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

    // Run the structural validator before resolving inputs: among
    // other rules it type-checks declared `default:` values against
    // `type:`, which `resolve_inputs` then relies on when carrying
    // defaults through unchanged. Without this, a buggy default
    // would only surface from `duhem validate`, not `duhem run`.
    if let Err(errs) = validate(&def) {
        let plural = if errs.len() == 1 { "" } else { "s" };
        eprintln!(
            "{}: {} validation error{plural}:",
            path.display(),
            errs.len()
        );
        for e in errs {
            eprintln!("  - {e}");
        }
        return ExitCode::FAILURE;
    }

    // CLI-side fail-fast per the typed-input-catalog spec: missing /
    // unknown / mistyped inputs error before `Engine::run` is called.
    let inputs = match resolve_inputs(&raw_inputs, &def.inputs) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    // Filter parse failures must surface before we boot a browser —
    // a typoed pattern shouldn't pay the Playwright launch cost.
    let check_filter = if raw_filter.is_empty() {
        None
    } else {
        match CliCheckFilter::parse(&raw_filter) {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    };

    let browser = match RunBrowser::launch(headed).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("browser: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut engine = Engine::new()
        .with_browser(browser)
        .with_definition_path(path.display().to_string());
    if let Some(dir) = evidence_dir {
        engine = engine.with_evidence_root(dir);
    }
    if let Some(f) = check_filter {
        engine = engine.with_filter(f);
    }
    let outcome = match engine.run_with_metadata(&def, inputs).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("engine: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut stdout = std::io::stdout().lock();
    if let Err(e) = reporter::render(reporter, &mut stdout, &outcome) {
        eprintln!("reporter: {e}");
        return ExitCode::FAILURE;
    }
    let _ = stdout.flush();

    match outcome.verdict.state {
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
            let value =
                yml_to_json(default).map_err(|e| format!("input `{name}`: default: {e}"))?;
            out.insert(name.clone(), value);
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
    use clap::Parser;

    /// Spec on #23 § Alignment "Headless default": `--headed` opts
    /// into a visible window; default stays headless. The runtime
    /// `RunBrowser::launch(headed: bool)` is keyed off this exact
    /// boolean, so the CLI → launch-arg translation is the smallest
    /// thing we can validate without booting Playwright.
    #[test]
    fn headed_flag_defaults_to_false_and_opts_in() {
        let default = Cli::try_parse_from(["duhem", "run", "v.yml"]).expect("parse");
        match default.cmd {
            Some(Cmd::Run { headed, .. }) => assert!(!headed, "default is headless"),
            other => panic!("expected Run, got {other:?}"),
        }
        let opted = Cli::try_parse_from(["duhem", "run", "v.yml", "--headed"]).expect("parse");
        match opted.cmd {
            Some(Cmd::Run { headed, .. }) => assert!(headed, "--headed opts in"),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn reporter_flag_parses_and_defaults_to_default() {
        let default = Cli::try_parse_from(["duhem", "run", "v.yml"]).expect("parse");
        match default.cmd {
            Some(Cmd::Run { reporter, .. }) => assert_eq!(reporter, ReporterArg::Default),
            _ => panic!("expected Run"),
        }
        for (s, want) in [
            ("default", ReporterArg::Default),
            ("quiet", ReporterArg::Quiet),
            ("json", ReporterArg::Json),
        ] {
            let parsed =
                Cli::try_parse_from(["duhem", "run", "v.yml", "--reporter", s]).expect("parse");
            match parsed.cmd {
                Some(Cmd::Run { reporter, .. }) => assert_eq!(reporter, want, "for `{s}`"),
                _ => panic!("expected Run"),
            }
        }
    }

    #[test]
    fn filter_flag_collects_repeated_values() {
        // Spec on #23: repeat `--filter` for OR. Verify the CLI surface
        // collects into the expected `Vec<String>` shape.
        let parsed = Cli::try_parse_from([
            "duhem",
            "run",
            "v.yml",
            "--filter",
            "AC-1",
            "--filter",
            "AC-2::AC-2.3",
        ])
        .expect("parse");
        match parsed.cmd {
            Some(Cmd::Run { filter, .. }) => {
                assert_eq!(filter, vec!["AC-1".to_string(), "AC-2::AC-2.3".to_string()]);
            }
            _ => panic!("expected Run"),
        }
    }

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

    #[test]
    fn object_default_with_non_string_keys_errors() {
        // YAML allows non-string mapping keys; JSON does not. Silently
        // dropping such entries would mutate the author's default —
        // surface it as a user-facing error from `resolve_inputs`.
        let d = decls("  flags: { type: object, default: { 1: x } }");
        let err = resolve_inputs(&raw(&[]), &d).unwrap_err();
        assert!(
            err.contains("flags") && err.contains("non-string"),
            "error names the input and the cause: {err}"
        );
    }
}
