//! Leaf-by-path manifest resolution (issue #384).
//!
//! Split out of `manifest.rs` to stay under the file-token budget.
//! Owns the part of leaf-by-path loading that is genuinely new: given
//! a leaf's *authored* definition, find its nearest ancestor root
//! manifest and merge in the `pages:` / `flows:` context it composes,
//! or — when no manifest exists — name exactly which `$pages.*` /
//! `call:` references the leaf can't resolve on its own. The manifest
//! walk, composition, and catalog-merge primitives this builds on live
//! in `manifest_compose.rs`; `manifest.rs` owns
//! [`crate::manifest::load_for_run`] (the dispatch entry point `duhem
//! run` calls).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::manifest::LoadError;
use crate::manifest_compose::{
    compose_manifest, merge_manifest_catalogs_into_leaf, walk_ancestors_for_manifest_excluding,
};
use crate::verification::{PageCatalog, VerificationDefinition};

/// Resolve a leaf loaded by explicit path (`-f`/`--file`, or an
/// explicit positional file — issue #384) against the nearest root
/// manifest, reusing the exact composition a manifest load applies to
/// every leaf it expands. Unlike a manifest load, this never loads,
/// parses, or executes any *other* `verifications:` entry the found
/// manifest lists — a leaf-by-path run stays scoped to the requested
/// leaf.
///
/// `def` must be the leaf's *authored* definition (parsed, flow
/// expansion not yet applied — expansion needs the merged catalog
/// this function produces).
///
/// Walks up from `leaf_path`'s directory using the same `.git`-capped
/// search [`crate::discover`] performs for a no-path `duhem run`.
/// Three outcomes:
///
/// - A manifest is found: its composed `pages:` / `flows:` catalog is
///   merged into `def` (leaf-local entries win, same rule a manifest
///   run applies to every leaf) and `call:` steps are expanded against
///   the combined catalog. Returns `Ok(Some(manifest_path))`.
/// - No manifest is found and `def` has no unresolved `$pages.*` or
///   `call:` reference: the leaf is genuinely self-contained and
///   expands unchanged. Returns `Ok(None)`.
/// - No manifest is found and `def` *does* reference manifest-only
///   content: fails with [`LoadError::UnresolvedWithoutRootManifest`],
///   naming every unresolved reference and where it would normally be
///   defined — not the flow expander's generic "unknown flow" error.
pub(crate) fn resolve_leaf_through_root_manifest(
    leaf_path: &Path,
    def: &mut VerificationDefinition,
) -> Result<Option<PathBuf>, LoadError> {
    let start = leaf_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // Exclude the leaf itself: Pattern A conventionally names a
    // self-contained leaf `duhem.yml`, so the walk's own starting
    // directory can contain a "candidate" that is actually the leaf
    // being resolved, not an ancestor manifest.
    match walk_ancestors_for_manifest_excluding(start, Some(leaf_path)) {
        Ok(manifest_path) => {
            let manifest_src =
                std::fs::read_to_string(&manifest_path).map_err(|e| LoadError::Io {
                    path: manifest_path.clone(),
                    source: e,
                })?;
            let manifest = compose_manifest(&manifest_path, &manifest_src)?;
            merge_manifest_catalogs_into_leaf(&manifest, def);
            crate::flows::validate_and_expand(def).map_err(|message| LoadError::InvalidFlows {
                path: leaf_path.to_path_buf(),
                message,
            })?;
            Ok(Some(manifest_path))
        }
        Err(LoadError::ManifestNotFound { .. }) => {
            let (flows, pages) = unresolved_leaf_references(def);
            if flows.is_empty() && pages.is_empty() {
                // Genuinely self-contained (spec requirement: a leaf
                // with no manifest-only references still runs
                // unchanged) — expand against its own local catalog,
                // today's behavior.
                crate::flows::validate_and_expand(def).map_err(|message| {
                    LoadError::InvalidFlows {
                        path: leaf_path.to_path_buf(),
                        message,
                    }
                })?;
                return Ok(None);
            }
            Err(LoadError::UnresolvedWithoutRootManifest {
                path: leaf_path.to_path_buf(),
                detail: describe_unresolved_refs(&flows, &pages, start),
            })
        }
        Err(e) => Err(e),
    }
}

/// Render the [`LoadError::UnresolvedWithoutRootManifest`] detail
/// sentence: which flows/pages are unresolved, and where they'd
/// normally be defined. Mirrors the voice of the CLI's existing
/// no-manifest `--profile` warning (`run_cmd.rs`) — name the gap,
/// don't just fail.
fn describe_unresolved_refs(flows: &[String], pages: &[String], searched_from: &Path) -> String {
    let mut named: Vec<String> = Vec::new();
    if !flows.is_empty() {
        named.push(format!(
            "flow{} {}",
            if flows.len() == 1 { "" } else { "s" },
            flows
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    if !pages.is_empty() {
        named.push(format!(
            "page locator{} {}",
            if pages.len() == 1 { "" } else { "s" },
            pages
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    format!(
        "references {} that this leaf does not define locally, and no root manifest was found walking up from `{}` (capped at a `.git` boundary) to supply them via `includes:` — declare them in this leaf's own `pages:` / `flows:` blocks, or run it from inside the manifest's directory tree",
        named.join(" and "),
        searched_from.display()
    )
}

/// Collect the `call:` flow names and `$pages.<page>.<element>`
/// references a leaf's own `pages:` / `flows:` blocks don't resolve.
/// Used only when no root manifest was found, to name precisely what a
/// manifest would otherwise have supplied (issue #384).
fn unresolved_leaf_references(def: &VerificationDefinition) -> (Vec<String>, Vec<String>) {
    let mut flows: BTreeSet<String> = BTreeSet::new();
    let mut pages: BTreeSet<String> = BTreeSet::new();

    fn visit_step(
        step: &crate::step::Step,
        def: &VerificationDefinition,
        flows: &mut BTreeSet<String>,
        pages: &mut BTreeSet<String>,
    ) {
        if let Some(name) = &step.call
            && !def.flows.contains_key(name)
        {
            flows.insert(name.clone());
        }
        collect_unresolved_page_refs(&step.with, &def.pages, pages);
    }

    for step in &def.setup {
        visit_step(step, def, &mut flows, &mut pages);
    }
    for criterion in &def.criteria {
        for check in &criterion.checks {
            for step in &check.steps {
                visit_step(step, def, &mut flows, &mut pages);
            }
            for assertion in &check.assertions {
                assertion.walk_exprs(|expr| {
                    collect_unresolved_page_paths(&expr.parsed, &def.pages, &mut pages);
                });
            }
        }
    }
    // A leaf-local flow body may itself reference a manifest page
    // (flows are hygiene-limited to `$params.*` / `$pages.*`).
    for flow in def.flows.values() {
        for step in &flow.steps {
            collect_unresolved_page_refs(&step.with, &def.pages, &mut pages);
        }
    }

    (flows.into_iter().collect(), pages.into_iter().collect())
}

fn collect_unresolved_page_refs(
    value: &serde_yml::Value,
    catalog: &PageCatalog,
    out: &mut BTreeSet<String>,
) {
    match value {
        serde_yml::Value::String(raw) if raw.trim_start().starts_with('$') => {
            if let Ok(expr) = crate::expr::parse(raw) {
                collect_unresolved_page_paths(&expr, catalog, out);
            }
        }
        serde_yml::Value::Sequence(values) => {
            for v in values {
                collect_unresolved_page_refs(v, catalog, out);
            }
        }
        serde_yml::Value::Mapping(values) => {
            for v in values.values() {
                collect_unresolved_page_refs(v, catalog, out);
            }
        }
        _ => {}
    }
}

fn collect_unresolved_page_paths(
    expr: &crate::expr::Expr,
    catalog: &PageCatalog,
    out: &mut BTreeSet<String>,
) {
    expr.walk_paths(|path| {
        if path.root != crate::expr::PathRoot::Pages {
            return;
        }
        let segments = path.segments();
        if segments.len() != 2 {
            out.insert(format!("$pages.{}", segments.join(".")));
            return;
        }
        let (page, element) = (&segments[0], &segments[1]);
        let known = catalog
            .get(page)
            .is_some_and(|entries| entries.contains_key(element));
        if !known {
            out.insert(format!("$pages.{page}.{element}"));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Loaded, load_for_run};

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn without_manifest_names_unresolved_references() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let leaf = write(
            tmp.path(),
            "orphan.yml",
            r#"
verification: orphan
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - call: open_login_page
          - uses: ui/assert-element
            with:
              locator: $pages.login.heading
              expected: visible
"#,
        );
        let err = load_for_run(&leaf).unwrap_err();
        match err {
            LoadError::UnresolvedWithoutRootManifest { detail, .. } => {
                assert!(detail.contains("open_login_page"), "{detail}");
                assert!(detail.contains("$pages.login.heading"), "{detail}");
                assert!(detail.contains("no root manifest was found"), "{detail}");
            }
            other => panic!("expected UnresolvedWithoutRootManifest, got {other:?}"),
        }
    }

    #[test]
    fn self_contained_leaf_is_unaffected() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let leaf = write(
            tmp.path(),
            "standalone.yml",
            r#"
verification: standalone
pages:
  home:
    heading: { text: Welcome }
flows:
  greet:
    steps:
      - uses: cli/invoke
        with: { command: printf, args: ["hi"] }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - call: greet
          - uses: ui/assert-element
            with:
              locator: $pages.home.heading
              expected: visible
"#,
        );
        let loaded = load_for_run(&leaf).expect("self-contained leaf loads without a manifest");
        match loaded {
            Loaded::Leaf { definition, .. } => {
                assert_eq!(definition.criteria[0].checks[0].steps.len(), 2);
            }
            _ => panic!("expected Leaf"),
        }
    }

    #[test]
    fn resolves_leaf_through_ancestor_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
pages:
  login:
    heading: { text: Sign in }
flows:
  greet:
    steps:
      - uses: cli/invoke
        with: { command: printf, args: ["hi"] }
verifications:
  - path: leaf-a.yml
"#,
        );
        let leaf_a = write(
            tmp.path(),
            "leaf-a.yml",
            r#"
verification: leaf-a
criteria:
  - id: AC-1
    description: uses the manifest's page and flow
    checks:
      - id: AC-1.1
        steps:
          - call: greet
          - uses: ui/assert-element
            with:
              locator: $pages.login.heading
              expected: visible
"#,
        );

        let loaded = load_for_run(&leaf_a).expect("resolves through the ancestor manifest");
        match loaded {
            Loaded::Leaf { definition, .. } => {
                let steps = &definition.criteria[0].checks[0].steps;
                // `call: greet` expanded into its one inner step; the
                // `ui/assert-element` step follows unchanged — its
                // `$pages.*` reference is a runtime lookup, not
                // something the loader expands away.
                assert_eq!(steps.len(), 2, "got {steps:?}");
                assert_eq!(steps[0].uses.as_deref(), Some("cli/invoke"));
                assert_eq!(steps[1].uses.as_deref(), Some("ui/assert-element"));
            }
            _ => panic!("expected Leaf"),
        }
    }

    #[test]
    fn stays_scoped_to_the_requested_leaf() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: leaf-a.yml
  - path: leaf-b.yml
"#,
        );
        let leaf_a = write(
            tmp.path(),
            "leaf-a.yml",
            "verification: leaf-a\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n        assertions: [\"true\"]\n",
        );
        // Deliberately invalid YAML (tab where YAML expects spaces). If
        // `load_for_run` ever read this file — even just to
        // structurally validate it while resolving leaf-a — the load
        // would fail.
        write(tmp.path(), "leaf-b.yml", "criteria:\n\t- id: AC-1\n");

        let loaded = load_for_run(&leaf_a).expect("leaf-b must never be read");
        match loaded {
            Loaded::Leaf { definition, .. } => {
                assert_eq!(definition.verification, "leaf-a");
            }
            _ => panic!("expected Leaf"),
        }
    }
}
