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
use duhem_schema::{VerificationDefinition, validate};

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
    let inputs = match parse_inputs(&raw_inputs) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
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
