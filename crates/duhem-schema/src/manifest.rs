//! `RootManifest` — `duhem.yml` at the root of a Verification
//! Definition tree.
//!
//! Lists or globs child Verification Definitions (Patterns B and C
//! from `docs/duhem-spec.md` §10.4). A manifest is *not* itself a
//! Verification Definition — no `criteria:`, no `setup:`. The loader
//! distinguishes manifest from leaf by which key is present.
//!
//! Spec on issue #49.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::verification::{SchemaError, VerificationDefinition};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootManifest {
    /// Document version. v1 today; future schema changes bump this so
    /// a manifest written against today's loader refuses to silently
    /// reinterpret a future shape.
    pub manifest_version: u32,
    /// Entries that resolve to leaf Verification Definitions. Order
    /// determines execution order across leaves.
    pub verifications: Vec<ManifestEntry>,
}

/// One entry in `verifications:`. An author writes either `path:` for
/// an explicit leaf (Pattern B) or `glob:` for a globbed expansion
/// (Pattern C). The two are mutually exclusive at the YAML level
/// (untagged enum), which keeps the wire shape unambiguous.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ManifestEntry {
    Path {
        /// Path relative to the manifest's parent directory. Absolute
        /// paths and `..` segments are rejected at load time.
        path: PathBuf,
    },
    Glob {
        /// Shell-style glob (`**`, `*`, `?`) expanded against the
        /// manifest's parent directory.
        glob: String,
    },
}

impl RootManifest {
    /// Parse a root manifest from YAML source. Does not resolve
    /// entries; call [`crate::load`] for the full discovery pipeline.
    pub fn from_yaml_str(src: &str) -> Result<Self, SchemaError> {
        serde_yml::from_str(src).map_err(SchemaError::from)
    }
}

/// Outcome of [`crate::load`] — either a single Verification
/// Definition (Pattern A; today's behavior) or a manifest that
/// expanded into N leaves (Patterns B / C).
#[derive(Debug)]
pub enum Loaded {
    Leaf {
        /// Absolute or as-supplied path of the leaf file.
        path: PathBuf,
        definition: VerificationDefinition,
    },
    Manifest {
        /// Path of the manifest file itself.
        manifest_path: PathBuf,
        manifest: RootManifest,
        /// Per-leaf `(path, definition)` pairs in the order they
        /// resolved from the manifest. Globs are pre-expanded and
        /// flattened.
        leaves: Vec<LoadedLeaf>,
        /// Non-fatal load-time warnings (e.g. a glob that matched
        /// nothing). CLI surfaces these to stderr.
        warnings: Vec<String>,
    },
}

#[derive(Debug)]
pub struct LoadedLeaf {
    /// Path on disk of the leaf file.
    pub path: PathBuf,
    /// Parsed Verification Definition.
    pub definition: VerificationDefinition,
}

/// Errors from [`crate::load`]. Distinct from [`SchemaError`] because
/// load-time problems span filesystem I/O, path discipline, and the
/// manifest/leaf-shape discriminator — not just YAML parsing.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: SchemaError,
    },
    /// A file declares both `verifications:` and `criteria:` — load
    /// cannot pick a shape.
    #[error(
        "{path}: cannot be both a root manifest and a Verification Definition (has both `verifications:` and `criteria:`)"
    )]
    AmbiguousShape { path: PathBuf },
    /// A file declares neither `verifications:` nor `criteria:`.
    #[error(
        "{path}: not a Verification Definition or root manifest (missing both `verifications:` and `criteria:`)"
    )]
    UnknownShape { path: PathBuf },
    #[error(
        "{manifest}: entry path `{entry}` is absolute; only paths relative to the manifest are allowed"
    )]
    AbsolutePath { manifest: PathBuf, entry: PathBuf },
    #[error("{manifest}: entry path `{entry}` escapes the manifest's parent directory via `..`")]
    PathEscape { manifest: PathBuf, entry: PathBuf },
    #[error("{manifest}: self-reference cycle on `{entry}`")]
    SelfReference { manifest: PathBuf, entry: PathBuf },
    #[error("{manifest}: glob pattern `{pattern}` is invalid: {source}")]
    InvalidGlob {
        manifest: PathBuf,
        pattern: String,
        #[source]
        source: glob::PatternError,
    },
    #[error("directory `{path}` has no `duhem.yml`")]
    DirectoryMissingManifest { path: PathBuf },
}

/// Resolve a CLI `path` argument to a [`Loaded`].
///
/// Dispatch rules (issue #49 § "CLI surface"):
///
/// - Directory → look for `<dir>/duhem.yml`.
/// - File with `verifications:` → root manifest; expand entries.
/// - File with `criteria:` → single leaf (today's behavior).
/// - File with both / neither → load-time error.
///
/// On a manifest, every resolved leaf is parsed eagerly; a malformed
/// leaf fails the whole load with a path-tagged error so authors see
/// the offending file.
pub fn load(path: &Path) -> Result<Loaded, LoadError> {
    let path = if path.is_dir() {
        let candidate = path.join("duhem.yml");
        if !candidate.is_file() {
            return Err(LoadError::DirectoryMissingManifest {
                path: path.to_path_buf(),
            });
        }
        candidate
    } else {
        path.to_path_buf()
    };

    let src = std::fs::read_to_string(&path).map_err(|e| LoadError::Io {
        path: path.clone(),
        source: e,
    })?;

    match classify_yaml(&src) {
        Shape::Manifest => load_manifest(&path, &src),
        Shape::Leaf => load_leaf(&path, &src).map(|definition| Loaded::Leaf {
            path: path.clone(),
            definition,
        }),
        Shape::Ambiguous => Err(LoadError::AmbiguousShape { path }),
        Shape::Unknown => Err(LoadError::UnknownShape { path }),
    }
}

enum Shape {
    Manifest,
    Leaf,
    Ambiguous,
    Unknown,
}

/// Top-level key sniff. We parse the YAML once as an untyped Mapping
/// and check which discriminator key is present. Both `verifications:`
/// and `criteria:` → ambiguous; neither → unknown.
fn classify_yaml(src: &str) -> Shape {
    let value: serde_yml::Value = match serde_yml::from_str(src) {
        Ok(v) => v,
        Err(_) => return Shape::Unknown,
    };
    let map = match value.as_mapping() {
        Some(m) => m,
        None => return Shape::Unknown,
    };
    let has_verifications = map.contains_key(serde_yml::Value::String("verifications".into()));
    let has_criteria = map.contains_key(serde_yml::Value::String("criteria".into()));
    match (has_verifications, has_criteria) {
        (true, true) => Shape::Ambiguous,
        (true, false) => Shape::Manifest,
        (false, true) => Shape::Leaf,
        (false, false) => Shape::Unknown,
    }
}

fn load_leaf(path: &Path, src: &str) -> Result<VerificationDefinition, LoadError> {
    VerificationDefinition::from_yaml_str(src).map_err(|e| LoadError::Yaml {
        path: path.to_path_buf(),
        source: e,
    })
}

fn load_manifest(manifest_path: &Path, src: &str) -> Result<Loaded, LoadError> {
    let manifest = RootManifest::from_yaml_str(src).map_err(|e| LoadError::Yaml {
        path: manifest_path.to_path_buf(),
        source: e,
    })?;
    let parent = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let manifest_canonical = canonical_or_self(manifest_path);

    let mut leaves: Vec<LoadedLeaf> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();

    for entry in &manifest.verifications {
        let resolved_paths = match entry {
            ManifestEntry::Path { path } => {
                validate_entry_path(manifest_path, path)?;
                vec![parent.join(path)]
            }
            ManifestEntry::Glob { glob: pattern } => {
                let joined = parent.join(pattern);
                let pattern_str = joined.to_string_lossy().into_owned();
                let matches = glob::glob(&pattern_str).map_err(|e| LoadError::InvalidGlob {
                    manifest: manifest_path.to_path_buf(),
                    pattern: pattern.clone(),
                    source: e,
                })?;
                let mut hits: Vec<PathBuf> = Vec::new();
                for p in matches.flatten() {
                    hits.push(p);
                }
                // Self-only globs (the glob expanded to *just* the
                // manifest itself) are a real cycle error: surface
                // them rather than silently warning.
                let only_self = !hits.is_empty()
                    && hits
                        .iter()
                        .all(|p| canonical_or_self(p) == manifest_canonical);
                if only_self {
                    return Err(LoadError::SelfReference {
                        manifest: manifest_path.to_path_buf(),
                        entry: PathBuf::from(pattern.clone()),
                    });
                }
                if hits.is_empty() {
                    warnings.push(format!(
                        "{}: glob `{}` matched no files",
                        manifest_path.display(),
                        pattern
                    ));
                }
                hits
            }
        };

        for leaf_path in resolved_paths {
            // Globs may surface the manifest itself; explicit
            // `path:` entries get the same self-reference check.
            if canonical_or_self(&leaf_path) == manifest_canonical {
                return Err(LoadError::SelfReference {
                    manifest: manifest_path.to_path_buf(),
                    entry: leaf_path.clone(),
                });
            }
            // Dedup repeated paths (e.g. two overlapping globs).
            // First occurrence wins, keeping authored order.
            let canon = canonical_or_self(&leaf_path);
            if !seen.insert(canon.clone()) {
                continue;
            }
            let src = std::fs::read_to_string(&leaf_path).map_err(|e| LoadError::Io {
                path: leaf_path.clone(),
                source: e,
            })?;
            // Each resolved leaf must be a Verification Definition.
            // A nested manifest is a real authoring mistake, not a
            // composition feature in v1.
            match classify_yaml(&src) {
                Shape::Leaf => {}
                Shape::Manifest => {
                    return Err(LoadError::UnknownShape {
                        path: leaf_path.clone(),
                    });
                }
                Shape::Ambiguous => {
                    return Err(LoadError::AmbiguousShape {
                        path: leaf_path.clone(),
                    });
                }
                Shape::Unknown => {
                    return Err(LoadError::UnknownShape {
                        path: leaf_path.clone(),
                    });
                }
            }
            let def = load_leaf(&leaf_path, &src)?;
            leaves.push(LoadedLeaf {
                path: leaf_path,
                definition: def,
            });
        }
    }

    Ok(Loaded::Manifest {
        manifest_path: manifest_path.to_path_buf(),
        manifest,
        leaves,
        warnings,
    })
}

fn validate_entry_path(manifest: &Path, entry: &Path) -> Result<(), LoadError> {
    if entry.is_absolute() {
        return Err(LoadError::AbsolutePath {
            manifest: manifest.to_path_buf(),
            entry: entry.to_path_buf(),
        });
    }
    // Reject any `..` segment in the entry — silently allowing
    // escapes would make `duhem run` reach files outside the
    // verifications directory, which is a real surprise vector.
    for component in entry.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(LoadError::PathEscape {
                manifest: manifest.to_path_buf(),
                entry: entry.to_path_buf(),
            });
        }
    }
    Ok(())
}

/// Best-effort canonicalization. Falls back to the input when the
/// file doesn't yet exist (e.g. unit tests using temp paths that may
/// or may not have been created). Identity comparison is "same
/// canonical path or, failing that, same lexical path."
fn canonical_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn parses_minimal_manifest() {
        let y = r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
  - glob: ./**/duhem.yml
"#;
        let m = RootManifest::from_yaml_str(y).expect("parse");
        assert_eq!(m.manifest_version, 1);
        assert_eq!(m.verifications.len(), 2);
        match &m.verifications[0] {
            ManifestEntry::Path { path } => assert_eq!(path, &PathBuf::from("./a/duhem.yml")),
            _ => panic!("expected path entry"),
        }
        match &m.verifications[1] {
            ManifestEntry::Glob { glob } => assert_eq!(glob, "./**/duhem.yml"),
            _ => panic!("expected glob entry"),
        }
    }

    #[test]
    fn manifest_rejects_unknown_top_level_field() {
        let y = "manifest_version: 1\nverifications: []\nfoo: bar\n";
        assert!(RootManifest::from_yaml_str(y).is_err());
    }

    #[test]
    fn load_pattern_b_resolves_explicit_paths() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(tmp.path(), "b/duhem.yml", LEAF_B);
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
  - path: ./b/duhem.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest {
                leaves, warnings, ..
            } => {
                assert_eq!(leaves.len(), 2);
                assert_eq!(leaves[0].definition.verification, "leaf-a");
                assert_eq!(leaves[1].definition.verification, "leaf-b");
                assert!(warnings.is_empty());
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn load_pattern_c_glob_resolves_leaves() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(tmp.path(), "b/duhem.yml", LEAF_B);
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - glob: ./*/duhem.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest {
                leaves, warnings, ..
            } => {
                let names: Vec<&str> = leaves
                    .iter()
                    .map(|l| l.definition.verification.as_str())
                    .collect();
                assert_eq!(names.len(), 2);
                assert!(names.contains(&"leaf-a"));
                assert!(names.contains(&"leaf-b"));
                assert!(warnings.is_empty());
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn zero_match_glob_warns_but_loads() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - glob: ./nope/*.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest {
                leaves, warnings, ..
            } => {
                assert!(leaves.is_empty());
                assert_eq!(warnings.len(), 1);
                assert!(warnings[0].contains("matched no files"), "{warnings:?}");
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn self_referential_path_is_cycle_error() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./duhem.yml
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::SelfReference { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn self_only_glob_is_cycle_error() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - glob: ./duhem.yml
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::SelfReference { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn absolute_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let abs = tmp.path().join("a").join("duhem.yml");
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(
            tmp.path(),
            "duhem.yml",
            &format!(
                r#"
manifest_version: 1
verifications:
  - path: {}
"#,
                abs.display()
            ),
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(matches!(err, LoadError::AbsolutePath { .. }), "got {err:?}");
    }

    #[test]
    fn parent_dir_escape_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ../duhem.yml
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(matches!(err, LoadError::PathEscape { .. }), "got {err:?}");
    }

    #[test]
    fn ambiguous_shape_is_load_error() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
verifications:
  - path: ./a/duhem.yml
criteria: []
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::AmbiguousShape { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn unknown_shape_is_load_error() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "duhem.yml", "verification: x\n");
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(matches!(err, LoadError::UnknownShape { .. }), "got {err:?}");
    }

    #[test]
    fn load_leaf_returns_leaf_variant() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(tmp.path(), "leaf.yml", LEAF_A);
        match load(&p).unwrap() {
            Loaded::Leaf { definition, .. } => assert_eq!(definition.verification, "leaf-a"),
            _ => panic!("expected Leaf"),
        }
    }

    #[test]
    fn directory_resolves_to_duhem_yml() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
"#,
        );
        let loaded = load(tmp.path()).unwrap();
        match loaded {
            Loaded::Manifest { leaves, .. } => assert_eq!(leaves.len(), 1),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn directory_without_manifest_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let err = load(tmp.path()).unwrap_err();
        assert!(
            matches!(err, LoadError::DirectoryMissingManifest { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn malformed_leaf_surfaces_offending_path() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", "criteria: not-a-sequence\n");
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        match err {
            LoadError::Yaml { path, .. } => {
                assert!(
                    path.to_string_lossy().contains("a/duhem.yml"),
                    "got {path:?}"
                );
            }
            other => panic!("expected Yaml, got {other:?}"),
        }
    }

    #[test]
    fn glob_dedups_with_explicit_path() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
  - glob: ./*/duhem.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest { leaves, .. } => assert_eq!(leaves.len(), 1, "dedup repeated leaf"),
            _ => panic!("expected Manifest"),
        }
    }
}
