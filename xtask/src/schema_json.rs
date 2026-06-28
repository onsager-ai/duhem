//! `schema-json` — emit (and drift-check) the published JSON Schema
//! derived from the `duhem-schema` Rust types (spec issue #133).
//!
//! Behavior:
//!
//!     cargo run -p xtask -- schema-json           # write schema/duhem.schema.json
//!     cargo run -p xtask -- schema-json --check   # fail if on-disk is stale
//!
//! The committed artifact at `schema/duhem.schema.json` is what
//! `# yaml-language-server: $schema=...` headers point at, so a struct
//! change that isn't regenerated drifts the editor experience away from
//! what `duhem validate` enforces. `--check` is the CI guard against
//! that drift.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

const SCHEMA_PATH: &str = "schema/duhem.schema.json";

pub fn run(args: Vec<String>) -> Result<()> {
    let check = args.iter().any(|a| a == "--check");
    let root = workspace_root()?;
    let out_path = root.join(SCHEMA_PATH);

    let generated = render();

    if check {
        let on_disk = std::fs::read_to_string(&out_path).with_context(|| {
            format!(
                "read {} (run `cargo run -p xtask -- schema-json` to generate it)",
                out_path.display()
            )
        })?;
        if on_disk != generated {
            bail!(
                "{} is out of date with the `duhem-schema` types.\n\
                 Run `cargo run -p xtask -- schema-json` to regenerate it.",
                SCHEMA_PATH
            );
        }
        eprintln!("schema-json: {SCHEMA_PATH} is up to date");
        return Ok(());
    }

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(&out_path, &generated)
        .with_context(|| format!("write {}", out_path.display()))?;
    eprintln!("schema-json: wrote {SCHEMA_PATH}");
    Ok(())
}

/// Render the schema as pretty-printed JSON with a trailing newline.
/// Deterministic: schemars 0.8 backs its maps with a `BTreeMap`, so
/// key order is stable across runs.
fn render() -> String {
    let value = duhem_schema::json_schema();
    let mut s = serde_json::to_string_pretty(&value).expect("schema serializes");
    s.push('\n');
    s
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .context("CARGO_MANIFEST_DIR not set; run via `cargo run -p xtask`")?;
    Ok(Path::new(&manifest)
        .parent()
        .ok_or_else(|| anyhow!("xtask manifest has no parent"))?
        .to_path_buf())
}
