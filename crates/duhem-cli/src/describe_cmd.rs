//! `duhem actions` and `duhem describe <uses>` — render the
//! machine-readable action contract (spec #247).
//!
//! On-demand, version-exact ground truth for an author (human or agent):
//! the `with:` fields an action accepts and the `outputs` it produces —
//! retrieval-by-tool, not docs-archaeology. Consumes the same
//! `duhem_actions::catalog()` that validate-time field checking uses.

use std::process::ExitCode;

use duhem_actions::{ActionContract, catalog, contract_for};

/// `duhem actions` — list the action catalog: one `uses` + summary per line.
pub(crate) fn run_actions() -> ExitCode {
    for c in catalog() {
        println!("{:<18}  {}", c.uses, c.summary);
    }
    ExitCode::SUCCESS
}

/// `duhem describe <uses>` — the full contract for one action.
pub(crate) fn run_describe(uses: &str) -> ExitCode {
    match contract_for(uses) {
        Some(c) => {
            print_contract(&c);
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("unknown action `{uses}`. Known actions (see `duhem actions`):");
            for c in catalog() {
                eprintln!("  {}", c.uses);
            }
            ExitCode::FAILURE
        }
    }
}

fn print_contract(c: &ActionContract) {
    println!("{}", c.uses);
    println!("  {}", c.summary);
    println!();
    println!("with:");
    for f in &c.with {
        let req = if f.required { "required" } else { "optional" };
        if f.enum_values.is_empty() {
            println!("  {:<14} {req}", f.name);
        } else {
            println!(
                "  {:<14} {req} — one of: {}",
                f.name,
                f.enum_values.join(", ")
            );
        }
    }
    println!();
    if c.outputs.is_empty() {
        println!("outputs: (none)");
    } else {
        println!("outputs: {}", c.outputs.join(", "));
        println!(
            "  (bound via `outputs: {{ <name>: <field> }}`, read as $steps.<id>.outputs.<name>)"
        );
    }
    println!();
    println!("example:");
    for line in c.example.lines() {
        println!("  {line}");
    }
}
