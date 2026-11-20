//! Scoped roots must share catalogs without inheriting another suite's run.
use std::path::{Path, PathBuf};

use duhem_schema::{LoadError, Loaded, RootManifest, load};

const LEAF: &str = "verification: scoped\ncriteria:\n  - id: AC-1\n    description: shared catalog\n    checks:\n      - id: AC-1.1\n        steps:\n          - call: shared\n        assertions: ['true']\n";
const CATALOG: &str = "pages:\n  login:\n    submit: {role: button, name: Shared}\nflows:\n  shared:\n    steps:\n      - uses: ui/click\n        with: {locator: $pages.login.submit}\n";

fn write(dir: &Path, name: &str, src: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, src).unwrap();
    path
}

fn scoped(dir: &Path, includes: &str) -> PathBuf {
    write(dir, "scope/check.yml", LEAF);
    write(
        dir,
        "scope/duhem.yml",
        &format!(
            "manifest_version: 1\nincludes: {includes}\nverifications: [{{path: check.yml}}]\n"
        ),
    )
}

#[test]
fn scoped_root_loads_catalog_without_loading_parent_siblings() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), ".git", "gitdir: irrelevant\n");
    // Ancestor manifests never supply configuration or verification entries.
    write(
        tmp.path(),
        "duhem.yml",
        "manifest_version: 999\nverifications: [{path: missing.yml}]\nprofiles: {parent: {url: parent}}\n",
    );
    write(tmp.path(), "shared.yml", CATALOG);
    let path = scoped(tmp.path(), "[{catalog: shared.yml}]");
    let Loaded::Manifest {
        manifest, leaves, ..
    } = load(&path).unwrap()
    else {
        panic!("expected manifest");
    };
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].definition.verification, "scoped");
    assert!(manifest.profiles.is_empty());
    assert!(manifest.provision.is_none());
    assert_eq!(manifest.flows.len(), 1);
    assert_eq!(
        leaves[0].definition.pages["login"]["submit"]["name"].as_str(),
        Some("Shared")
    );
    assert_eq!(
        leaves[0].definition.criteria[0].checks[0].steps[0]
            .uses
            .as_deref(),
        Some("ui/click")
    );
}

#[test]
fn root_first_then_first_catalog_and_leaf_local_precedence() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), ".git", "gitdir: irrelevant\n");
    write(
        tmp.path(),
        "duhem.yml",
        "manifest_version: 1\nverifications: []\n",
    );
    write(tmp.path(), "first.yml", CATALOG);
    write(
        tmp.path(),
        "second.yml",
        &CATALOG.replace("Shared", "Second"),
    );
    let path = scoped(tmp.path(), "[{catalog: first.yml}, {catalog: second.yml}]");
    let mut src = std::fs::read_to_string(&path).unwrap();
    src.push_str("pages: {login: {submit: {role: button, name: Root}}}\n");
    std::fs::write(&path, src).unwrap();
    write(
        tmp.path(),
        "scope/check.yml",
        &format!("{LEAF}pages: {{login: {{submit: {{role: button, name: Leaf}}}}}}\n"),
    );
    let Loaded::Manifest {
        manifest, leaves, ..
    } = load(&path).unwrap()
    else {
        panic!("expected manifest");
    };
    assert_eq!(
        manifest.pages["login"]["submit"]["name"].as_str(),
        Some("Root")
    );
    assert_eq!(
        leaves[0].definition.pages["login"]["submit"]["name"].as_str(),
        Some("Leaf")
    );

    let path = scoped(tmp.path(), "[{catalog: first.yml}, {catalog: second.yml}]");
    let Loaded::Manifest { manifest, .. } = load(&path).unwrap() else {
        panic!("expected manifest");
    };
    assert_eq!(
        manifest.pages["login"]["submit"]["name"].as_str(),
        Some("Shared")
    );
}

#[test]
fn nested_partial_uses_invoked_roots_anchor() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), ".git", "gitdir: irrelevant\n");
    write(
        tmp.path(),
        "duhem.yml",
        "manifest_version: 1\nverifications: []\n",
    );
    write(tmp.path(), "shared.yml", CATALOG);
    write(
        tmp.path(),
        "scope/config/local.yml",
        "includes: [{catalog: shared.yml}]\n",
    );
    let path = scoped(tmp.path(), "[config/local.yml]");
    assert!(load(&path).is_ok());
}

#[test]
fn no_git_uses_invoked_roots_own_directory_despite_ancestor_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "duhem.yml",
        "manifest_version: 1\nverifications: []\n",
    );
    write(tmp.path(), "shared.yml", "invalid: ancestor catalog\n");
    write(tmp.path(), "scope/shared.yml", CATALOG);
    let path = scoped(tmp.path(), "[{catalog: shared.yml}]");
    assert!(load(&path).is_ok());
}

#[test]
fn intermediate_manifests_do_not_move_repository_anchor() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".git")).unwrap();
    write(
        tmp.path(),
        "duhem.yml",
        "manifest_version: 1\nverifications: []\n",
    );
    write(tmp.path(), "shared.yml", CATALOG);
    write(
        tmp.path(),
        "nested/shared.yml",
        &CATALOG.replace("Shared", "Intermediate"),
    );
    let path = scoped(&tmp.path().join("nested"), "[{catalog: shared.yml}]");
    for intermediate in [Some("nested/duhem.yml"), Some("nested/.duhem.yml"), None] {
        if let Some(name) = intermediate {
            write(tmp.path(), name, "manifest_version: 1\nverifications: []\n");
        }
        let Loaded::Manifest { manifest, .. } = load(&path).unwrap() else {
            panic!("expected manifest");
        };
        assert_eq!(
            manifest.pages["login"]["submit"]["name"].as_str(),
            Some("Shared")
        );
        if let Some(name) = intermediate {
            std::fs::remove_file(tmp.path().join(name)).unwrap();
        }
    }
}

#[test]
fn catalog_paths_and_documents_are_restricted() {
    let tmp = tempfile::tempdir().unwrap();
    let path = scoped(tmp.path(), "[{catalog: ../shared.yml}]");
    assert!(matches!(load(&path), Err(LoadError::PathEscape { .. })));
    let absolute = tmp.path().join("shared.yml");
    let path = scoped(
        tmp.path(),
        &format!("[{{catalog: '{}' }}]", absolute.display()),
    );
    assert!(matches!(load(&path), Err(LoadError::AbsolutePath { .. })));

    for field in [
        "verifications: []",
        "provision: {}",
        "profiles: {}",
        "includes: []",
        "manifest_version: 1",
    ] {
        write(tmp.path(), "scope/.git", "boundary");
        write(
            tmp.path(),
            "scope/shared.yml",
            &format!("{CATALOG}{field}\n"),
        );
        let path = scoped(tmp.path(), "[{catalog: shared.yml}]");
        assert!(
            matches!(load(&path), Err(LoadError::Yaml { .. })),
            "accepted {field}"
        );
    }
    assert!(RootManifest::from_yaml_str("manifest_version: 1\nincludes: [{catalog: shared.yml, unknown: true}]\nverifications: []\n").is_err());
}

#[test]
fn missing_catalog_is_an_error_and_mixed_includes_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "scope/.git", "boundary");
    let path = scoped(tmp.path(), "[{catalog: missing.yml}]");
    assert!(matches!(load(&path), Err(LoadError::Io { .. })));
    let src =
        "manifest_version: 1\nincludes: [local.yml, {catalog: shared.yml}]\nverifications: []\n";
    let manifest = RootManifest::from_yaml_str(src).unwrap();
    let serialized = serde_yml::to_string(&manifest).unwrap();
    assert_eq!(manifest, RootManifest::from_yaml_str(&serialized).unwrap());
}

#[test]
fn catalog_terminal_counts_toward_include_depth_limit() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "scope/.git", "boundary");
    write(tmp.path(), "scope/shared.yml", CATALOG);
    write(tmp.path(), "scope/one.yml", "includes: [two.yml]\n");
    write(tmp.path(), "scope/two.yml", "includes: [three.yml]\n");
    write(
        tmp.path(),
        "scope/three.yml",
        "includes: [{catalog: shared.yml}]\n",
    );
    let path = scoped(tmp.path(), "[one.yml]");
    assert!(matches!(
        load(&path),
        Err(LoadError::IncludeDepthExceeded { max: 3, .. })
    ));
    write(
        tmp.path(),
        "scope/two.yml",
        "includes: [{catalog: shared.yml}]\n",
    );
    assert!(load(&path).is_ok());
}

#[cfg(unix)]
#[test]
fn symlink_cannot_escape_catalog_anchor() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "outside.yml", CATALOG);
    let path = scoped(tmp.path(), "[{catalog: shared.yml}]");
    write(tmp.path(), "scope/.git", "boundary");
    std::os::unix::fs::symlink(
        tmp.path().join("outside.yml"),
        tmp.path().join("scope/shared.yml"),
    )
    .unwrap();
    assert!(matches!(load(&path), Err(LoadError::CatalogEscaped { .. })));
}

#[test]
fn worked_example_loads_both_whole_suite_and_independent_scope() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verifications/catalog-includes-example");
    for path in [root.join("duhem.yml"), root.join("scoped/duhem.yml")] {
        let Loaded::Manifest { leaves, .. } = load(&path).unwrap() else {
            panic!("expected manifest");
        };
        assert_eq!(leaves.len(), 1);
    }
}
