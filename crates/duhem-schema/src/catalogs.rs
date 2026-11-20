//! Suite-anchored, catalog-only imports for independently runnable scopes (#379).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::manifest::{LoadError, RootManifest};
use crate::manifest_path::{canonical_or_self, validate_entry_path};
use crate::verification::{FlowCatalog, PageCatalog};

/// Intentionally excludes provisioning, profiles, verifications and recursion.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    #[serde(default)]
    pages: PageCatalog,
    #[serde(default)]
    flows: FlowCatalog,
}

/// The nearest repository root supplies an anchor, independent of manifests.
/// Keep this fixed for all imports in one composition, including nested partials.
fn suite_root(root_manifest: &Path) -> PathBuf {
    let parent = root_manifest.parent().unwrap_or_else(|| Path::new("."));
    for ancestor in parent.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor.to_path_buf();
        }
    }
    parent.to_path_buf()
}

pub(crate) fn merge_catalog(
    declaring_path: &Path,
    root_manifest: &Path,
    catalog: &Path,
    effective: &mut RootManifest,
) -> Result<(), LoadError> {
    validate_entry_path(declaring_path, catalog)?;
    let root = suite_root(root_manifest);
    let path = root.join(catalog);
    let canonical = std::fs::canonicalize(&path).map_err(|source| LoadError::Io {
        path: path.clone(),
        source,
    })?;
    if !canonical.starts_with(canonical_or_self(&root)) {
        return Err(LoadError::CatalogEscaped {
            manifest: declaring_path.to_path_buf(),
            entry: path,
        });
    }
    let src = std::fs::read_to_string(&canonical).map_err(|source| LoadError::Io {
        path: path.clone(),
        source,
    })?;
    let incoming: Catalog = serde_yml::from_str(&src).map_err(|source| LoadError::Yaml {
        path,
        source: source.into(),
    })?;
    for (page, elements) in incoming.pages {
        let target = effective.pages.entry(page).or_default();
        for (element, locator) in elements {
            target.entry(element).or_insert(locator);
        }
    }
    for (name, flow) in incoming.flows {
        effective.flows.entry(name).or_insert(flow);
    }
    Ok(())
}
