//! Manifest discovery and composition primitives shared by a full
//! manifest load ([`crate::manifest::load_manifest`]) and leaf-by-path
//! resolution ([`crate::leaf_context`], issue #384).
//!
//! Split out of `manifest.rs` to stay under the file-token budget —
//! these are self-contained (no dependency on the leaf-expansion loop
//! or the `Loaded`/`LoadedLeaf` result shapes that stay in
//! `manifest.rs`).

use std::path::{Path, PathBuf};

use crate::manifest::{
    LoadError, MANIFEST_CANDIDATES, RootManifest, SUPPORTED_MANIFEST_VERSION,
    profile_name_well_formed,
};
use crate::manifest_path::canonical_or_self;
use crate::verification::VerificationDefinition;

/// Walk `start` and its ancestors (capped at a `.git` boundary) for
/// the first [`MANIFEST_CANDIDATES`] hit. This is the search
/// [`crate::discover`] performs when no `path` argument is given,
/// factored out so leaf-by-path resolution (issue #384) reuses the
/// exact same walk instead of a second, potentially-drifting
/// implementation — changing one without the other would silently
/// desync what "no manifest found" means between a no-path `duhem
/// run` and an explicit leaf path.
pub(crate) fn walk_ancestors_for_manifest(start: &Path) -> Result<PathBuf, LoadError> {
    walk_ancestors_for_manifest_excluding(start, None)
}

/// Same walk as [`walk_ancestors_for_manifest`], but a candidate that
/// resolves to `exclude` (compared canonically) is treated as absent
/// and the walk keeps looking — the rest of that directory's
/// candidates, then its ancestors.
///
/// Needed for leaf-by-path resolution (issue #384): Pattern A names a
/// self-contained leaf `duhem.yml` (exactly what `duhem init`
/// scaffolds), which is itself one of the [`MANIFEST_CANDIDATES`]
/// names. Starting the walk at that leaf's own directory would
/// otherwise "find" the leaf as its own ancestor manifest and fail
/// trying to parse a Verification Definition as a `RootManifest` — a
/// leaf is never its own manifest.
pub(crate) fn walk_ancestors_for_manifest_excluding(
    start: &Path,
    exclude: Option<&Path>,
) -> Result<PathBuf, LoadError> {
    let exclude_canonical = exclude.map(canonical_or_self);
    let mut searched: Vec<PathBuf> = Vec::new();
    let mut dir = Some(start);
    while let Some(current) = dir {
        for name in MANIFEST_CANDIDATES {
            let candidate = current.join(name);
            if candidate.is_file() {
                if exclude_canonical.as_ref() != Some(&canonical_or_self(&candidate)) {
                    return Ok(candidate);
                }
                // This candidate *is* the excluded leaf — keep
                // checking the other candidate names in this
                // directory before moving on to its ancestors.
                continue;
            }
            searched.push(candidate);
        }
        // Repo-boundary cap: a `.git` entry stops the walk *after* the
        // boundary directory itself is probed (matching git's own
        // discovery — the repo root often holds the manifest).
        if current.join(".git").exists() {
            break;
        }
        dir = current.parent();
    }
    Err(LoadError::ManifestNotFound { searched })
}

/// Parse and fully compose a root manifest from `src` — version check,
/// `includes:` resolution (spec #67, root-wins), and the
/// manifest-level structural checks (`inputs:`, `project:`,
/// `profiles:`) — **without** expanding any `verifications:` entry
/// into leaves. [`crate::manifest::load_manifest`] calls this before
/// its leaf-expansion loop; leaf-by-path resolution (issue #384) calls
/// it directly so a leaf's `$pages.*` / flow references resolve
/// against the exact same composed catalog a full manifest run would
/// use, without loading or executing any sibling `verifications:`
/// entry.
pub(crate) fn compose_manifest(manifest_path: &Path, src: &str) -> Result<RootManifest, LoadError> {
    let mut manifest = RootManifest::from_yaml_str(src).map_err(|e| LoadError::Yaml {
        path: manifest_path.to_path_buf(),
        source: e,
    })?;
    if manifest.manifest_version != SUPPORTED_MANIFEST_VERSION {
        return Err(LoadError::UnsupportedManifestVersion {
            path: manifest_path.to_path_buf(),
            found: manifest.manifest_version,
            supported: SUPPORTED_MANIFEST_VERSION,
        });
    }
    // Compose `includes:` (spec #67) before any structural checks so
    // the rest of this function operates on the *effective* manifest —
    // root values plus include-supplied fills. Root-wins: an include
    // only fills keys the root left absent. `manifest` is mutated in
    // place into the merged result.
    let manifest_parent = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let root_includes = manifest.includes.clone();
    let mut chain = vec![canonical_or_self(manifest_path)];
    crate::includes::resolve_includes(
        manifest_path,
        manifest_parent,
        &root_includes,
        &mut manifest,
        &mut chain,
        0,
    )?;
    crate::manifest_inputs::validate(manifest_path, &manifest.inputs)?;
    // Suite-wide `project:` discipline (#191): exactly one non-empty
    // coordinate. Same load-time home as the other structural checks.
    if let Some(project) = &manifest.project
        && let Err(msg) = project.check()
    {
        return Err(LoadError::BadProjectDecl {
            path: manifest_path.to_path_buf(),
            message: msg,
        });
    }
    // Named-profiles discipline (spec #68): well-formed names,
    // non-empty key maps. Cheap structural checks at load time, the
    // same place the manifest-version and path checks live.
    for (name, keys) in &manifest.profiles {
        if !profile_name_well_formed(name) {
            return Err(LoadError::MalformedProfileName {
                manifest: manifest_path.to_path_buf(),
                name: name.clone(),
            });
        }
        if keys.is_empty() {
            return Err(LoadError::EmptyProfile {
                manifest: manifest_path.to_path_buf(),
                name: name.clone(),
            });
        }
    }
    Ok(manifest)
}

/// Overlay a composed manifest's `pages:` / `flows:` catalog onto a
/// leaf definition — leaf-local entries win, merged element-by-element
/// for pages and by name for flows. Shared by manifest leaf-expansion
/// and leaf-by-path resolution (issue #384) so both apply identical
/// composition rules.
pub(crate) fn merge_manifest_catalogs_into_leaf(
    manifest: &RootManifest,
    def: &mut VerificationDefinition,
) {
    for (page, elements) in &manifest.pages {
        let target = def.pages.entry(page.clone()).or_default();
        for (element, locator) in elements {
            target
                .entry(element.clone())
                .or_insert_with(|| locator.clone());
        }
    }
    for (name, flow) in &manifest.flows {
        def.flows
            .entry(name.clone())
            .or_insert_with(|| flow.clone());
    }
}
