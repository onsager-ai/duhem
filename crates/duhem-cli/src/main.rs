//! `duhem` — the command-line entry point.
//!
//! Phase-0 skeleton. The free CLI binary (`docs/duhem-spec.md` §13)
//! ultimately offers `init`, `validate`, and `run`; subcommands land
//! in `spec(cli): duhem init / validate / run skeletons`. The
//! `validate` subcommand exists today as a preview to prove the
//! `duhem-schema` type surface is reachable from the binary.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
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
    ///
    /// Preview; the full surface lands in
    /// `spec(cli): duhem init / validate / run skeletons`.
    Validate {
        /// Path to a `.yml` Verification Definition.
        path: PathBuf,
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
