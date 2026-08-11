//! Path-discipline helpers shared by manifest loading and includes.

use std::path::{Path, PathBuf};

use crate::manifest::LoadError;

pub(crate) fn validate_glob_pattern(manifest: &Path, pattern: &str) -> Result<(), LoadError> {
    let candidate = Path::new(pattern);
    if candidate.is_absolute()
        || candidate
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(LoadError::UnconstrainedGlob {
            manifest: manifest.to_path_buf(),
            pattern: pattern.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn is_under(root: &Path, candidate: &Path) -> bool {
    canonical_or_self(candidate).starts_with(root)
}

pub(crate) fn validate_entry_path(manifest: &Path, entry: &Path) -> Result<(), LoadError> {
    if entry.is_absolute() {
        return Err(LoadError::AbsolutePath {
            manifest: manifest.to_path_buf(),
            entry: entry.to_path_buf(),
        });
    }
    if entry
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(LoadError::PathEscape {
            manifest: manifest.to_path_buf(),
            entry: entry.to_path_buf(),
        });
    }
    Ok(())
}

/// Best-effort canonicalization for identity and containment checks.
pub(crate) fn canonical_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
