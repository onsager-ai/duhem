//! `includes:` composition (spec #67) — pulling shared + local
//! manifest fragments into a root manifest under the root-wins rule.
//!
//! Split out of `manifest.rs` to keep that file under the per-file
//! token budget; the loader (`crate::manifest::load`) calls
//! [`resolve_includes`] before its structural checks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::environment::Environment;
use crate::manifest::{
    LoadError, ManifestEntry, RootManifest, canonical_or_self, validate_entry_path,
};
use crate::verification::SchemaError;

/// An `includes:` target — a manifest fragment composed into a root
/// manifest (spec #67). Structurally a [`RootManifest`] with every
/// field optional and **no** `manifest_version:`: the document version
/// is the root manifest's responsibility, so an include carrying a
/// `manifest_version:` is rejected (`deny_unknown_fields` makes it an
/// unknown key). An include may itself declare `includes:`, which are
/// resolved before its own fields are merged (depth-first).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartialRootManifest {
    /// Nested includes, resolved depth-first relative to *this*
    /// include's parent directory. See [`RootManifest::includes`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<PathBuf>,
    /// Shared suite environment to fill in when the root manifest
    /// declares none. See [`RootManifest::environment`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
    /// Named environment configs, overlaid key-by-key under the
    /// root-wins rule. See [`RootManifest::environments`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(
        with = "std::collections::BTreeMap<String, std::collections::BTreeMap<String, serde_json::Value>>"
    )]
    pub environments: BTreeMap<String, BTreeMap<String, serde_yml::Value>>,
    /// Verification entries concatenated onto the root manifest's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verifications: Vec<ManifestEntry>,
}

impl PartialRootManifest {
    /// Parse a partial manifest (an `includes:` target) from YAML
    /// source. A `manifest_version:` key — or any other key not on the
    /// partial — is a parse error.
    pub fn from_yaml_str(src: &str) -> Result<Self, SchemaError> {
        serde_yml::from_str(src).map_err(SchemaError::from)
    }
}

/// Maximum nesting depth for `includes:` (spec #67). The root manifest
/// is depth 0, its direct includes depth 1, and so on; an include at a
/// depth beyond this is a [`LoadError::IncludeDepthExceeded`]. Bounds
/// the recursion and keeps composition reviewable.
pub const MAX_INCLUDE_DEPTH: usize = 3;

/// Resolve an `includes:` list (spec #67) and merge each target into
/// `effective` under the root-wins rule.
///
/// `declaring_path` is the manifest/partial file that lists these
/// includes; `declaring_parent` is the directory its include paths
/// resolve against. `chain` holds the canonical paths currently being
/// resolved (seeded with the root manifest) so a re-entry is a
/// [`LoadError::IncludeCycle`] naming both ends. `depth` is the depth
/// of `declaring_path` itself (0 for the root manifest); each include
/// sits at `depth + 1` and may not exceed [`MAX_INCLUDE_DEPTH`].
///
/// Order is depth-first: an include's own `includes:` are resolved
/// before its fields are merged, and includes are processed in
/// declared order. Combined with `or_insert`-style filling, this makes
/// "the first include to supply an absent key wins" deterministic.
pub(crate) fn resolve_includes(
    declaring_path: &Path,
    declaring_parent: &Path,
    includes: &[PathBuf],
    effective: &mut RootManifest,
    chain: &mut Vec<PathBuf>,
    depth: usize,
) -> Result<(), LoadError> {
    for include in includes {
        // Same path discipline as `verifications:` entries: no
        // absolute paths, no `..` escape out of the manifest tree.
        validate_entry_path(declaring_path, include)?;
        let include_path = declaring_parent.join(include);
        let include_canonical = canonical_or_self(&include_path);

        // Cycle: the target is already somewhere on the active chain
        // (including the root manifest itself).
        if chain.contains(&include_canonical) {
            return Err(LoadError::IncludeCycle {
                manifest: declaring_path.to_path_buf(),
                target: include_path,
            });
        }
        if depth + 1 > MAX_INCLUDE_DEPTH {
            return Err(LoadError::IncludeDepthExceeded {
                manifest: declaring_path.to_path_buf(),
                target: include_path,
                max: MAX_INCLUDE_DEPTH,
            });
        }

        let inc_src = std::fs::read_to_string(&include_path).map_err(|e| LoadError::Io {
            path: include_path.clone(),
            source: e,
        })?;
        // An include must parse as a partial manifest. A
        // `manifest_version:` (or any other unexpected key) is rejected
        // by `deny_unknown_fields`.
        let partial =
            PartialRootManifest::from_yaml_str(&inc_src).map_err(|e| LoadError::Yaml {
                path: include_path.clone(),
                source: e,
            })?;

        // Resolve this include's own includes first (depth-first),
        // then merge its own fields.
        let inc_parent = include_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        chain.push(include_canonical);
        resolve_includes(
            &include_path,
            &inc_parent,
            &partial.includes,
            effective,
            chain,
            depth + 1,
        )?;
        chain.pop();

        merge_partial(effective, &partial);
    }
    Ok(())
}

/// Root-wins merge of one include (`incoming`) into `effective`.
///
/// Each optional/scalar field is filled only when `effective` still
/// lacks it; nested maps (`environments:`) overlay key-by-key, again
/// filling only absent leaf keys; list-shaped fields (`verifications:`,
/// `includes:`) are concatenated. Adding a future field to the merge
/// (e.g. `defaults:`) is a one-block addition here.
fn merge_partial(effective: &mut RootManifest, incoming: &PartialRootManifest) {
    // Scalar/optional fields: root-wins, fill-if-absent.
    if effective.environment.is_none() {
        effective.environment = incoming.environment.clone();
    }
    // Nested map: overlay key-by-key, still root-wins per leaf key.
    for (name, keys) in &incoming.environments {
        let slot = effective.environments.entry(name.clone()).or_default();
        for (key, value) in keys {
            slot.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    // List-shaped fields: concatenate (root's already present first).
    effective
        .verifications
        .extend(incoming.verifications.iter().cloned());
    effective.includes.extend(incoming.includes.iter().cloned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Loaded, load};

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }

    const LEAF_A: &str = r#"
verification: leaf-a
criteria:
  - id: AC-1
    description: trivial
    checks:
      - id: AC-1.1
        assertions:
          - "true"
"#;

    const LEAF_B: &str = r#"
verification: leaf-b
criteria:
  - id: AC-1
    description: trivial
    checks:
      - id: AC-1.1
        assertions:
          - "true"
"#;

    // ----- includes: composition (spec #67) -----

    #[test]
    fn absent_includes_round_trips_with_no_includes_key() {
        // Additive guarantee: a manifest without `includes:` behaves
        // byte-for-byte as before — no `includes` key on the wire.
        let y = "manifest_version: 1\nverifications: []\n";
        let m = RootManifest::from_yaml_str(y).expect("parse");
        assert!(m.includes.is_empty());
        let out = serde_yml::to_string(&m).unwrap();
        assert!(!out.contains("includes"), "got: {out}");
    }

    #[test]
    fn includes_merge_is_root_wins_then_first_include_wins() {
        // staging.db_url is declared by the root → root wins.
        // staging.base_url is declared by both includes → the first
        // (a.yml) wins. staging.workers is only in b.yml → b supplies
        // it. Exercises all three precedence rules at once.
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "leaf/duhem.yml", LEAF_A);
        write(
            tmp.path(),
            ".duhem.a.yml",
            r#"
environments:
  staging:
    base_url: from-a
    db_url: from-a
"#,
        );
        write(
            tmp.path(),
            ".duhem.b.yml",
            r#"
environments:
  staging:
    base_url: from-b
    workers: 3
"#,
        );
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
includes:
  - ./.duhem.a.yml
  - ./.duhem.b.yml
environments:
  staging:
    db_url: from-root
verifications:
  - path: ./leaf/duhem.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest { manifest, .. } => {
                let staging = &manifest.environments["staging"];
                assert_eq!(staging["db_url"].as_str(), Some("from-root"));
                assert_eq!(staging["base_url"].as_str(), Some("from-a"));
                assert_eq!(staging["workers"].as_u64(), Some(3));
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn includes_concatenate_verifications_in_declared_order() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(tmp.path(), "b/duhem.yml", LEAF_B);
        write(
            tmp.path(),
            ".duhem.shared.yml",
            r#"
verifications:
  - path: ./b/duhem.yml
"#,
        );
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
includes:
  - ./.duhem.shared.yml
verifications:
  - path: ./a/duhem.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest { leaves, .. } => {
                let names: Vec<&str> = leaves
                    .iter()
                    .map(|l| l.definition.verification.as_str())
                    .collect();
                // Root's entries first, then the include's.
                assert_eq!(names, vec!["leaf-a", "leaf-b"]);
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn include_with_manifest_version_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            ".duhem.shared.yml",
            "manifest_version: 1\nverifications: []\n",
        );
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
includes:
  - ./.duhem.shared.yml
verifications: []
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        match err {
            LoadError::Yaml { path, source } => {
                assert!(
                    path.to_string_lossy().contains(".duhem.shared.yml"),
                    "names the offending include: {path:?}"
                );
                assert!(
                    source.to_string().contains("manifest_version"),
                    "error mentions the rejected key: {source}"
                );
            }
            other => panic!("expected Yaml, got {other:?}"),
        }
    }

    #[test]
    fn include_cycle_reports_both_paths() {
        let tmp = tempfile::tempdir().unwrap();
        // duhem.yml -> shared.yml -> duhem.yml. The second hop is a
        // cycle back onto the root manifest.
        write(
            tmp.path(),
            ".duhem.shared.yml",
            r#"
includes:
  - ./duhem.yml
"#,
        );
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
includes:
  - ./.duhem.shared.yml
verifications: []
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        let msg = err.to_string();
        match &err {
            LoadError::IncludeCycle { manifest, target } => {
                assert!(
                    manifest.to_string_lossy().contains(".duhem.shared.yml"),
                    "declaring file named: {manifest:?}"
                );
                assert!(
                    target.to_string_lossy().contains("duhem.yml"),
                    "cycle target named: {target:?}"
                );
                // Both ends of the cycle appear in the message.
                assert!(
                    msg.contains(".duhem.shared.yml"),
                    "msg names declarer: {msg}"
                );
                assert!(msg.contains("duhem.yml"), "msg names target: {msg}");
            }
            other => panic!("expected IncludeCycle, got {other:?}"),
        }
    }

    #[test]
    fn include_depth_four_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        // root(0) -> i1(1) -> i2(2) -> i3(3) -> i4(4, rejected).
        write(tmp.path(), "i4.yml", "verifications: []\n");
        write(tmp.path(), "i3.yml", "includes:\n  - ./i4.yml\n");
        write(tmp.path(), "i2.yml", "includes:\n  - ./i3.yml\n");
        write(tmp.path(), "i1.yml", "includes:\n  - ./i2.yml\n");
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
includes:
  - ./i1.yml
verifications: []
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::IncludeDepthExceeded { max: 3, .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn include_depth_three_is_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        // root(0) -> i1(1) -> i2(2) -> i3(3) — exactly at the limit.
        write(tmp.path(), "i3.yml", "verifications: []\n");
        write(tmp.path(), "i2.yml", "includes:\n  - ./i3.yml\n");
        write(tmp.path(), "i1.yml", "includes:\n  - ./i2.yml\n");
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
includes:
  - ./i1.yml
verifications: []
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).expect("depth 3 loads");
        assert!(matches!(loaded, Loaded::Manifest { .. }));
    }

    #[test]
    fn absolute_include_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let abs = tmp.path().join(".duhem.shared.yml");
        write(tmp.path(), ".duhem.shared.yml", "verifications: []\n");
        write(
            tmp.path(),
            "duhem.yml",
            &format!(
                r#"
manifest_version: 1
includes:
  - {}
verifications: []
"#,
                abs.display()
            ),
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(matches!(err, LoadError::AbsolutePath { .. }), "got {err:?}");
    }

    #[test]
    fn parent_dir_escape_include_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
includes:
  - ../.duhem.shared.yml
verifications: []
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(matches!(err, LoadError::PathEscape { .. }), "got {err:?}");
    }

    #[test]
    fn partial_manifest_rejects_manifest_version_directly() {
        // Unit-level check on the partial parser itself.
        assert!(PartialRootManifest::from_yaml_str("manifest_version: 1\n").is_err());
        // And it accepts an all-optional document (even empty).
        assert!(PartialRootManifest::from_yaml_str("{}\n").is_ok());
    }

    #[test]
    fn worked_example_loads_green() {
        // The in-tree worked example at
        // `verifications/includes-example/` — one shared include + one
        // local include + a leaf — must load without error.
        let example = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../verifications/includes-example")
            .join("duhem.yml");
        let loaded = load(&example).expect("worked example loads");
        match loaded {
            Loaded::Manifest {
                leaves, manifest, ..
            } => {
                assert_eq!(leaves.len(), 1, "one leaf");
                assert!(
                    manifest.environments.contains_key("staging"),
                    "merged environments present: {:?}",
                    manifest.environments
                );
            }
            _ => panic!("expected Manifest"),
        }
    }
}
