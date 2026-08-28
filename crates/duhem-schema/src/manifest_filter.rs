//! Run-scoping operations over an already resolved root manifest.

use std::path::Path;

use crate::manifest::{LoadError, Loaded};
use crate::manifest_path::{canonical_or_self, is_under};

/// Restrict a manifest-resolved run to entries beneath an explicitly
/// requested directory (issue #464).
///
/// Filtering happens after manifest composition and glob expansion so
/// every retained leaf keeps the ancestor manifest's defaults, inputs,
/// profiles, pages, and flows. A direct leaf target is returned unchanged.
/// An empty manifest intersection is an error: a run that verifies
/// nothing must not report success.
pub fn filter_loaded_to_directory(
    mut loaded: Loaded,
    directory: &Path,
) -> Result<Loaded, LoadError> {
    let Loaded::Manifest {
        manifest_path,
        leaves,
        ..
    } = &mut loaded
    else {
        return Ok(loaded);
    };

    let directory_canonical = canonical_or_self(directory);
    leaves.retain(|leaf| is_under(&directory_canonical, &leaf.path));
    if leaves.is_empty() {
        return Err(LoadError::DirectoryMatchesNoVerifications {
            directory: directory.to_path_buf(),
            manifest: manifest_path.clone(),
        });
    }
    Ok(loaded)
}
