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
                if exclude_canonical.as_ref() == Some(&canonical_or_self(&candidate)) {
                    // This candidate *is* the excluded leaf — keep
                    // checking the other candidate names in this
                    // directory before moving on to its ancestors.
                    continue;
                }
                // Leaf-by-path resolution (`exclude: Some(_)`, called
                // from `leaf_context`) additionally requires the
                // candidate to actually be *shaped* like a root
                // manifest. Pattern A conventionally names any
                // self-contained leaf `duhem.yml` — the same name a
                // real root manifest uses — so an ancestor directory
                // can hold a candidate-named file that is itself just
                // another leaf (e.g. a sibling regression suite), not
                // a root manifest at all. Composing such a file as a
                // `RootManifest` fails outright (it has no
                // `verifications:` key), so treat it as absent here —
                // same as the excluded-leaf case above — and keep
                // walking its ancestors for a real one.
                //
                // The no-path walk (`exclude: None`, `discover`'s
                // case) does not apply this check: it hands the first
                // name match back unclassified and `load` sorts out
                // its shape afterward (including running it standalone
                // if it turns out to be a leaf) — today's behavior,
                // left intact.
                if exclude.is_some() && !candidate_is_root_manifest(&candidate) {
                    continue;
                }
                return Ok(candidate);
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

/// True when `path`'s content is shaped like a root manifest (a
/// top-level `verifications:` key) rather than a leaf Verification
/// Definition (`criteria:`) or something unparsable. Mirrors the
/// `verifications:` / `criteria:` discriminator `manifest::classify_yaml`
/// uses to dispatch `load` / `load_for_run`, reimplemented narrowly as
/// a yes/no probe here: a read or parse failure isn't this walk's
/// problem to report, it just means the candidate doesn't qualify and
/// the walk keeps looking exactly as if the file were absent.
fn candidate_is_root_manifest(path: &Path) -> bool {
    let Ok(src) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_yml::from_str::<serde_yml::Value>(&src) else {
        return false;
    };
    value
        .as_mapping()
        .is_some_and(|map| map.contains_key(serde_yml::Value::String("verifications".into())))
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
