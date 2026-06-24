//! `duhem validate` — parse + structurally validate one Verification
//! Definition. Lives in its own module so `main.rs` stays under the
//! per-file token budget.

use duhem_schema::{VerificationDefinition, validate};

/// Resolve a CLI path to a VD file: a directory resolves to
/// `<dir>/duhem.yml` (the canonical VD filename), matching how
/// `duhem run`'s loader dispatches a directory (`duhem_schema::load`).
/// Keeps `validate <dir>` and `run <dir>` consistent.
pub(crate) fn resolve_vd_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if path.is_dir() {
        let candidate = path.join("duhem.yml");
        if candidate.is_file() {
            Ok(candidate)
        } else {
            Err(format!("{}: directory has no duhem.yml", path.display()))
        }
    } else {
        Ok(path.to_path_buf())
    }
}

pub(crate) fn run_validate(path: &std::path::Path) -> Result<(), String> {
    let path = resolve_vd_path(path)?;
    let path = path.as_path();
    let src = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let v = VerificationDefinition::from_yaml_str(&src).map_err(|e| match e.location() {
        Some(loc) => format!(
            "{}:{}:{}: [schema v{}] {e}",
            path.display(),
            loc.line(),
            loc.column(),
            duhem_schema::SCHEMA_VERSION
        ),
        None => format!(
            "{}: [schema v{}] {e}",
            path.display(),
            duhem_schema::SCHEMA_VERSION
        ),
    })?;
    validate(&v).map_err(|errs| {
        let plural = if errs.len() == 1 { "" } else { "s" };
        // Preamble names the schema version the file was validated
        // against — when authors hit a validation error, the next
        // question is "which schema?", and a downstream VD that pinned
        // a different version needs to see the mismatch.
        let mut s = format!(
            "[schema v{}] {} validation error{plural}:",
            duhem_schema::SCHEMA_VERSION,
            errs.len()
        );
        for e in errs {
            s.push_str("\n  - ");
            s.push_str(&e.to_string());
        }
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_vd_path_handles_dir_file_and_missing() {
        use std::fs;
        let tmp = std::env::temp_dir().join(format!("duhem-rvp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Directory with a duhem.yml resolves to it.
        let vd = tmp.join("duhem.yml");
        fs::write(&vd, "verification: x\ncriteria: []\n").unwrap();
        assert_eq!(resolve_vd_path(&tmp).unwrap(), vd);

        // A file path passes through unchanged.
        assert_eq!(resolve_vd_path(&vd).unwrap(), vd);

        // A directory without a duhem.yml errors clearly.
        let empty = tmp.join("empty");
        fs::create_dir_all(&empty).unwrap();
        let err = resolve_vd_path(&empty).unwrap_err();
        assert!(err.contains("no duhem.yml"), "got: {err}");

        let _ = fs::remove_dir_all(&tmp);
    }
}
