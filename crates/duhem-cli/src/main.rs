//! `duhem` — the command-line entry point.
//!
//! Phase-0 skeleton. The free CLI binary (`docs/duhem-spec.md` §13)
//! ultimately offers `init`, `validate`, and `run`; subcommands land in
//! `spec(cli): duhem init / validate / run skeletons`. This file
//! exists today so the binary links and the workspace builds.

use clap::Parser;

/// Duhem — holistic verification for AI-delivered software.
#[derive(Debug, Parser)]
#[command(name = "duhem", version, about, long_about = None)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
