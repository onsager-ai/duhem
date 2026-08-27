//! `duhem validate` — parse + structurally validate a Verification
//! Definition *or* a root manifest (and every leaf it expands to).
//!
//! Routes through the same polymorphic `duhem_schema::discover` +
//! `load` pipeline that `duhem run` uses, so a path that resolves to a
//! manifest is validated as a manifest (manifest_version, entry/path
//! discipline, profiles/defaults/includes, glob expansion) plus
//! each resolved leaf — instead of being mis-parsed as a leaf and
//! failing with `unknown field manifest_version` (#150).
//!
//! Lives in its own module so `main.rs` stays under the per-file token
//! budget.

use std::path::Path;

use duhem_schema::{
    LoadError, Loaded, SCHEMA_VERSION, ValidationError, validate_with_action_catalog,
};

/// Validate the target a `duhem validate [path]` invocation points at.
///
/// `path` is the positional argument (`None` when omitted, in which
/// case discovery walks the cwd and its ancestors for a manifest, the
/// same as `duhem run`). Returns the success line to print on stdout
/// (`OK` for a leaf; an `OK — …` summary naming the manifest and its
/// leaf count for a manifest) or a structured, file-tagged error.
pub(crate) fn run_validate(path: Option<&Path>) -> Result<String, String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot determine current directory: {e}"))?;
    let target = duhem_schema::discover(path, &cwd).map_err(format_load_error)?;
    let loaded = duhem_schema::load(&target).map_err(format_load_error)?;

    match loaded {
        // Single leaf: `OK` on success; failures carry the same source
        // provenance as a manifest leaf and `duhem run`.
        Loaded::Leaf { path, definition } => {
            validate_with_action_catalog(&definition, &crate::contract_check::catalog_outputs)
                .map_err(|errs| format_validation_errors(Some(&path), &errs))?;
            let cerrs = crate::contract_check::field_errors(&definition);
            if !cerrs.is_empty() {
                return Err(format!(
                    "[schema v{SCHEMA_VERSION}] action-contract check failed:\n  {}",
                    cerrs.join("\n  ")
                ));
            }
            // Non-fatal authoring lints (spec #267) go to stderr; the
            // verdict stays `OK` so an existing VD never breaks on them.
            for w in crate::contract_check::lint_warnings(&definition) {
                eprintln!("warning: {w}");
            }
            Ok("OK".to_string())
        }
        // Manifest: `load` already enforced the manifest-structural
        // rules (manifest_version, entry/glob path discipline,
        // profile names, include cycles) and eagerly parsed every
        // leaf. All that's left is the per-leaf *structural* validation.
        // Each failing leaf is reported with its path so the author sees
        // the offending file; every leaf is checked so one save → one
        // punch list.
        Loaded::Manifest {
            manifest_path,
            leaves,
            warnings,
            ..
        } => {
            // Non-fatal load warnings (e.g. a glob that matched nothing)
            // go to stderr, mirroring `duhem run`.
            for w in &warnings {
                eprintln!("warning: {w}");
            }
            let mut errors: Vec<String> = Vec::new();
            for leaf in &leaves {
                if let Err(errs) = validate_with_action_catalog(
                    &leaf.definition,
                    &crate::contract_check::catalog_outputs,
                ) {
                    errors.push(format_validation_errors(Some(&leaf.path), &errs));
                }
                let cerrs = crate::contract_check::field_errors(&leaf.definition);
                if !cerrs.is_empty() {
                    errors.push(format!(
                        "{}: action-contract check failed:\n  {}",
                        leaf.path.display(),
                        cerrs.join("\n  ")
                    ));
                }
            }
            if !errors.is_empty() {
                return Err(errors.join("\n"));
            }
            // Non-fatal authoring lints (spec #267), tagged with the
            // offending leaf so a manifest run points at the file.
            for leaf in &leaves {
                for w in crate::contract_check::lint_warnings(&leaf.definition) {
                    eprintln!("warning: {}: {w}", leaf.path.display());
                }
            }
            let n = leaves.len();
            // User-facing vocabulary is "verification", never the
            // internal manifest-tree "leaf" (#305 ride-along).
            let plural = if n == 1 {
                "verification"
            } else {
                "verifications"
            };
            Ok(format!(
                "OK — validated manifest {} + {n} {plural}",
                manifest_path.display()
            ))
        }
    }
}

/// Render a [`LoadError`] for stderr. A leaf/manifest YAML parse error
/// keeps today's location-aware, schema-versioned preamble byte-for-byte
/// (`<path>:<line>:<col>: [schema vX] <err>`); every other load-time
/// problem (path discipline, manifest_version, include cycle, …) is
/// prefixed with the schema version, matching `duhem run`.
fn format_load_error(e: LoadError) -> String {
    match &e {
        LoadError::Yaml { path, source } => match source.location() {
            Some(loc) => format!(
                "{}:{}:{}: [schema v{SCHEMA_VERSION}] {source}",
                path.display(),
                loc.line(),
                loc.column(),
            ),
            None => format!("{}: [schema v{SCHEMA_VERSION}] {source}", path.display()),
        },
        _ => format!("[schema v{SCHEMA_VERSION}] {e}"),
    }
}

/// Format each structural validation error as an independent diagnostic.
/// Exact marks use the same `<path>:<line>:<col>:` prefix as YAML parse
/// failures; unlocated variants retain a file-level `<path>:` fallback.
pub(crate) fn format_validation_errors(path: Option<&Path>, errs: &[ValidationError]) -> String {
    errs.iter()
        .map(|error| match (path, error.location()) {
            (Some(path), Some(location)) => format!(
                "{}:{}:{}: [schema v{SCHEMA_VERSION}] validation error: {error}",
                path.display(),
                location.line,
                location.column,
            ),
            (Some(path), None) => format!(
                "{}: [schema v{SCHEMA_VERSION}] validation error: {error}",
                path.display()
            ),
            (None, _) => format!("[schema v{SCHEMA_VERSION}] validation error: {error}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_a_leaf_file() {
        let tmp = tempfile::tempdir().unwrap();
        let leaf = tmp.path().join("v.yml");
        std::fs::write(
            &leaf,
            "verification: x\ncriteria:\n  - id: AC-1\n    description: d\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n",
        )
        .unwrap();
        assert_eq!(run_validate(Some(&leaf)).unwrap(), "OK");
    }

    #[test]
    fn validates_a_terse_leaf_with_implicit_outputs() {
        // A step that binds no `outputs:` yet asserts over a contract
        // output validates via the CLI's contract-aware resolver
        // (spec #267) — the real end-to-end wiring, not a stub.
        let tmp = tempfile::tempdir().unwrap();
        let leaf = tmp.path().join("v.yml");
        std::fs::write(
            &leaf,
            r#"verification: x
criteria:
  - id: AC-1
    description: d
    checks:
      - id: AC-1.1
        description: d
        steps:
          - id: home
            uses: api/call
            with: { method: GET, url: "https://example.com" }
        assertions:
          - $steps.home.outputs.status == 200
"#,
        )
        .unwrap();
        assert_eq!(run_validate(Some(&leaf)).unwrap(), "OK");
    }

    #[test]
    fn validates_a_manifest_directory() {
        // The exact case that errored before #150: a directory whose
        // `duhem.yml` is a root manifest.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("leaf.yml"),
            "verification: x\ncriteria:\n  - id: AC-1\n    description: d\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("duhem.yml"),
            "manifest_version: 1\nverifications:\n  - path: leaf.yml\n",
        )
        .unwrap();
        let msg = run_validate(Some(tmp.path())).unwrap();
        assert!(msg.starts_with("OK"), "got: {msg}");
        assert!(
            msg.contains("1 verification"),
            "names the verification count: {msg}"
        );
        // The pre-#150 mis-parse symptom must be gone.
        assert!(!msg.contains("manifest_version"), "got: {msg}");
    }

    #[test]
    fn manifest_with_broken_leaf_names_the_leaf() {
        let tmp = tempfile::tempdir().unwrap();
        // Parses fine (has `criteria:`) but fails structural validation
        // (empty criteria → NoCriteria).
        std::fs::write(
            tmp.path().join("bad.yml"),
            "verification: x\ncriteria: []\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("duhem.yml"),
            "manifest_version: 1\nverifications:\n  - path: bad.yml\n",
        )
        .unwrap();
        let err = run_validate(Some(tmp.path())).unwrap_err();
        assert!(err.contains("bad.yml"), "names the offending leaf: {err}");
        assert!(err.contains("no criteria"), "carries the cause: {err}");
    }

    #[test]
    fn leaf_validation_error_has_path_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let leaf = tmp.path().join("v.yml");
        std::fs::write(&leaf, "verification: x\ncriteria: []\n").unwrap();
        let err = run_validate(Some(&leaf)).unwrap_err();
        assert!(
            err.starts_with(&format!("{}: [schema v", leaf.display())),
            "leaf fallback names its file: {err}"
        );
        assert!(!err.contains(":0:0:"), "never fabricate a zero span: {err}");
    }

    #[test]
    fn unsupported_manifest_version_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("duhem.yml"),
            "manifest_version: 99\nverifications: []\n",
        )
        .unwrap();
        let err = run_validate(Some(tmp.path())).unwrap_err();
        assert!(err.contains("unsupported manifest_version"), "got: {err}");
    }
}
