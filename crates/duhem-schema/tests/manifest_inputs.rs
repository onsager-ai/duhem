//! Manifest input declaration contract tests (spec #354).
//!
//! These exercise the public loader rather than serde in isolation:
//! declaration validation, include composition, and advisory warnings
//! are all loader responsibilities shared by `duhem run` and
//! `duhem validate`.

use std::path::{Path, PathBuf};

use duhem_schema::{LoadError, Loaded, RootManifest, load};

const LEAF: &str = r#"
verification: manifest input consumer
inherits: [password]
criteria:
  - id: AC-1
    description: The inherited credential is available to the check.
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;

fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn manifest_inputs_round_trip_and_absence_stays_wire_empty() {
    let declared = RootManifest::from_yaml_str(
        r#"
manifest_version: 1
inputs:
  password: { type: string, env: CRAWLAB_PASSWORD, secret: true }
verifications: []
"#,
    )
    .unwrap();
    assert!(declared.inputs["password"].secret);
    assert_eq!(
        declared.inputs["password"].env.as_deref(),
        Some("CRAWLAB_PASSWORD")
    );

    let absent = RootManifest::from_yaml_str("manifest_version: 1\nverifications: []\n").unwrap();
    assert!(absent.inputs.is_empty());
    let encoded = serde_yml::to_string(&absent).unwrap();
    assert!(
        !encoded.contains("inputs:"),
        "optional field must not perturb old manifests: {encoded}"
    );
}

#[test]
fn unused_manifest_input_warns_without_failing_load() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "leaf.yml",
        &LEAF.replace("inherits: [password]\n", ""),
    );
    let manifest = write(
        tmp.path(),
        "duhem.yml",
        r#"
manifest_version: 1
inputs:
  password: { type: string, env: CRAWLAB_PASSWORD, secret: true }
verifications:
  - path: leaf.yml
"#,
    );

    match load(&manifest).expect("warning is non-fatal") {
        Loaded::Manifest {
            leaves, warnings, ..
        } => {
            assert_eq!(leaves.len(), 1);
            assert_eq!(warnings.len(), 1);
            assert!(
                warnings[0].contains("password")
                    && warnings[0].contains("not inherited by any verification"),
                "warning is actionable: {warnings:?}"
            );
        }
        Loaded::Leaf { .. } => panic!("expected manifest"),
    }
}

#[test]
fn manifest_secret_with_default_fails_validation() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "leaf.yml", LEAF);
    let manifest = write(
        tmp.path(),
        "duhem.yml",
        r#"
manifest_version: 1
inputs:
  password: { type: string, secret: true, default: committed }
verifications:
  - path: leaf.yml
"#,
    );

    let err = load(&manifest).unwrap_err();
    assert!(
        matches!(err, LoadError::InvalidManifestInputs { .. }),
        "manifest declaration validation owns the failure: {err:?}"
    );
    let message = err.to_string();
    assert!(
        message.contains("password")
            && message.contains("secret: true")
            && message.contains("default:"),
        "same rule and wording as a leaf declaration: {message}"
    );
}

#[test]
fn included_manifest_inputs_merge_root_first_by_name() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "leaf.yml", LEAF);
    write(
        tmp.path(),
        ".duhem.shared.yml",
        r#"
inputs:
  password: { type: string, env: SHARED_PASSWORD, secret: true }
  username: { type: string }
"#,
    );
    let manifest = write(
        tmp.path(),
        "duhem.yml",
        r#"
manifest_version: 1
includes: [.duhem.shared.yml]
inputs:
  password: { type: string, env: ROOT_PASSWORD, secret: true }
verifications:
  - path: leaf.yml
"#,
    );

    match load(&manifest).unwrap() {
        Loaded::Manifest { manifest, .. } => {
            assert_eq!(
                manifest.inputs["password"].env.as_deref(),
                Some("ROOT_PASSWORD")
            );
            assert!(manifest.inputs.contains_key("username"));
        }
        Loaded::Leaf { .. } => panic!("expected manifest"),
    }
}
