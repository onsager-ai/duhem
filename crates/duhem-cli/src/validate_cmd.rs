//! `duhem validate` — parse + structurally validate a Verification
//! Definition *or* a root manifest (and every leaf it expands to).
//!
//! Routes through the same polymorphic `duhem_schema::discover` +
//! `load` pipeline that `duhem run` uses, so a path that resolves to a
//! manifest is validated as a manifest (manifest_version, entry/path
//! discipline, environments/defaults/includes, glob expansion) plus
//! each resolved leaf — instead of being mis-parsed as a leaf and
//! failing with `unknown field manifest_version` (#150). A leaf path
//! keeps today's behavior byte-for-byte.
//!
//! Lives in its own module so `main.rs` stays under the per-file token
//! budget.

use std::path::Path;

use duhem_schema::{LoadError, Loaded, SCHEMA_VERSION, ValidationError, validate};

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
        // Single leaf: today's behavior, byte-for-byte — `OK` on
        // success, the un-prefixed validation-error preamble on failure.
        Loaded::Leaf { path, definition } => {
            validate(&definition).map_err(|errs| format_validation_errors(None, &errs))?;
            let cerrs = crate::contract_check::field_errors(&definition);
            if !cerrs.is_empty() {
                return Err(format!(
                    "[schema v{SCHEMA_VERSION}] action-contract check failed:\n  {}",
                    cerrs.join("\n  ")
                ));
            }
            let _ = path;
            Ok("OK".to_string())
        }
        // Manifest: `load` already enforced the manifest-structural
        // rules (manifest_version, entry/glob path discipline,
        // environment names, include cycles) and eagerly parsed every
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
                if let Err(errs) = validate(&leaf.definition) {
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
            let n = leaves.len();
            let plural = if n == 1 { "leaf" } else { "leaves" };
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

/// Format a leaf's structural validation errors. With `path` (a
/// manifest leaf) the preamble names the offending file; without it (a
/// single-leaf validate) the preamble is today's byte-for-byte form.
fn format_validation_errors(path: Option<&Path>, errs: &[ValidationError]) -> String {
    let plural = if errs.len() == 1 { "" } else { "s" };
    // Preamble names the schema version the file was validated against —
    // when authors hit a validation error, the next question is "which
    // schema?", and a downstream VD that pinned a different version needs
    // to see the mismatch.
    let mut s = match path {
        Some(p) => format!(
            "{}: [schema v{SCHEMA_VERSION}] {} validation error{plural}:",
            p.display(),
            errs.len()
        ),
        None => format!(
            "[schema v{SCHEMA_VERSION}] {} validation error{plural}:",
            errs.len()
        ),
    };
    for e in errs {
        s.push_str("\n  - ");
        s.push_str(&e.to_string());
    }
    s
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
        assert!(msg.contains("1 leaf"), "names the leaf count: {msg}");
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
    fn leaf_validation_error_omits_path_prefix() {
        // Byte-for-byte: a single-leaf validate keeps the un-prefixed
        // preamble (no `<path>:` lead) it had before #150.
        let tmp = tempfile::tempdir().unwrap();
        let leaf = tmp.path().join("v.yml");
        std::fs::write(&leaf, "verification: x\ncriteria: []\n").unwrap();
        let err = run_validate(Some(&leaf)).unwrap_err();
        assert!(
            err.starts_with("[schema v"),
            "leaf preamble is un-prefixed: {err}"
        );
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
