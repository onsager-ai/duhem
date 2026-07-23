//! Duhem repo-local tasks. Currently:
//!
//!     cargo run -p xtask -- check-file-budget        # bound per-file token cost
//!     cargo run -p xtask -- count-tokens <file>      # one-file token count
//!     cargo run -p xtask -- schema-drift             # docs/spec.md §10 ↔ code
//!     cargo run -p xtask -- schema-changelog-check   # CHANGELOG.md touch gate
//!     cargo run -p xtask -- schema-json [--check]    # emit/verify JSON Schema
//!     cargo run -p xtask -- skill-scrub              # published skills ↔ no internal vocab
//!     cargo run -p xtask -- dx-drift [--mode=warn|fail]  # DX surfaces ↔ product currency
//!
//! `check-file-budget` enforces a per-file token budget on every `.rs`
//! file under `crates/` and `xtask/src/`. The vocab is `tiktoken`'s
//! `o200k_base`, vendored at `xtask/assets/o200k_base.tiktoken` for
//! offline determinism. Ported from `onsager-ai/onsager` per
//! `docs/duhem-spec.md` Phase-0 plan (issue #5).
//!
//! `schema-drift` and `schema-changelog-check` enforce the schema-
//! versioning discipline from the spec issue that introduced
//! `duhem_schema::SCHEMA_VERSION`.

mod action_reference;
mod check_file_budget;
mod dx_drift;
mod schema_changelog_check;
mod schema_drift;
mod schema_json;
mod skill_scrub;

use std::process::ExitCode;

use anyhow::{Result, anyhow};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();

    let result: Result<()> = match cmd.as_deref() {
        Some("check-file-budget") => check_file_budget::run(args.collect()),
        Some("count-tokens") => check_file_budget::run_count(args.collect()),
        Some("schema-drift") => schema_drift::run(args.collect()),
        Some("schema-changelog-check") => schema_changelog_check::run(args.collect()),
        Some("schema-json") => schema_json::run(args.collect()),
        Some("action-reference") => action_reference::run(args.collect()),
        Some("skill-scrub") => skill_scrub::run(args.collect()),
        Some("dx-drift") => dx_drift::run(args.collect()),
        Some(other) => Err(anyhow!("unknown subcommand: {other}")),
        None => Err(anyhow!(
            "usage:\n  cargo run -p xtask -- check-file-budget [--mode=warn|fail] [--budget=N]\n  cargo run -p xtask -- count-tokens <file>\n  cargo run -p xtask -- schema-drift\n  cargo run -p xtask -- schema-changelog-check\n  cargo run -p xtask -- schema-json [--check]\n  cargo run -p xtask -- action-reference [--check]\n  cargo run -p xtask -- skill-scrub\n  cargo run -p xtask -- dx-drift [--mode=warn|fail]"
        )),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e:#}");
            ExitCode::FAILURE
        }
    }
}
