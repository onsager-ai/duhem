//! Offline suggestions only: never execute or modify a project's test setup.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use duhem_schema::native::{NativeReport, NativeReportFormat};
use duhem_schema::{NativeLayer, NativeRun};

pub(super) struct Proposal {
    pub root: PathBuf,
    pub runs: Vec<NativeRun>,
}

pub(super) fn detect(target: &Path) -> Option<Proposal> {
    let absolute = fs::canonicalize(target).ok()?;
    for root in absolute.ancestors() {
        let runs = detect_at(root);
        if !runs.is_empty() {
            return Some(Proposal {
                root: root.to_path_buf(),
                runs,
            });
        }
        // Do not escape a project into an enclosing repository or filesystem.
        if [
            ".git",
            "Cargo.toml",
            "package.json",
            "pyproject.toml",
            "go.mod",
        ]
        .iter()
        .any(|name| root.join(name).exists())
        {
            break;
        }
    }
    None
}

fn run(id: &str, command: &[&str], report: Option<&str>) -> NativeRun {
    NativeRun {
        id: id.into(),
        layer: Some(NativeLayer::Unit),
        command: command.iter().map(|arg| (*arg).into()).collect(),
        cwd: None,
        env: BTreeMap::new(),
        timeout: None,
        required: true,
        expect: None,
        report: report.map(|path| NativeReport {
            format: NativeReportFormat::Junit,
            path: path.into(),
        }),
    }
}

fn read_toml(path: &Path) -> Option<toml::Value> {
    fs::read_to_string(path).ok()?.parse().ok()
}

fn detect_at(root: &Path) -> Vec<NativeRun> {
    let mut runs = Vec::new();
    if root.join("Cargo.toml").is_file() {
        let nextest = read_toml(&root.join(".config/nextest.toml"));
        // https://nexte.st/docs/machine-readable/junit/ documents this exact
        // profile path and target/nextest/ci/junit.xml output location.
        let junit = nextest.as_ref().and_then(|value| {
            value
                .get("profile")?
                .get("ci")?
                .get("junit")?
                .get("path")?
                .as_str()
        });
        runs.push(if junit == Some("junit.xml") {
            run(
                "rust-unit",
                &["cargo", "nextest", "run", "--profile", "ci"],
                Some("target/nextest/ci/junit.xml"),
            )
        } else {
            run("rust-unit", &["cargo", "test", "--workspace"], None)
        });
    }
    let package = fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    if package
        .as_ref()
        .and_then(|value| value.get("scripts")?.get("test")?.as_str())
        .is_some()
    {
        let manager = if root.join("pnpm-lock.yaml").is_file() {
            "pnpm"
        } else if root.join("yarn.lock").is_file() {
            "yarn"
        } else {
            "npm"
        };
        runs.push(run("js-unit", &[manager, "test"], None));
    }
    let pyproject = read_toml(&root.join("pyproject.toml"));
    if root.join("pytest.ini").is_file()
        || root.join("conftest.py").is_file()
        || pyproject
            .as_ref()
            .and_then(|value| {
                value
                    .get("tool")?
                    .get("pytest")?
                    .get("ini_options")?
                    .as_table()
            })
            .is_some()
    {
        runs.push(run(
            "python-unit",
            &["pytest", "--junitxml=duhem-pytest.xml"],
            Some("duhem-pytest.xml"),
        ));
    }
    if root.join("go.mod").is_file() {
        runs.push(run("go-unit", &["go", "test", "./..."], None));
    }
    runs
}

impl Proposal {
    pub fn manifest(&self, target: &Path) -> String {
        let leaf = fs::canonicalize(target)
            .expect("created target exists")
            .strip_prefix(&self.root)
            .expect("detected root contains target")
            .join("duhem.yml")
            .to_string_lossy()
            .replace('\\', "/");
        let value = serde_json::json!({
            "manifest_version": 1,
            "native": self.runs,
            "verifications": [leaf],
        });
        format!(
            "# Native entries proposed from test setup detection; review before running.\n{}",
            serde_yml::to_string(&value).expect("manifest serializes")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init::{InitArgs, Kind, Pattern, run_with_prompt};

    fn scaffold(root: &Path, pattern: Pattern) -> super::super::init::InitOutcome {
        run_with_prompt(
            InitArgs {
                path: Some(root.join("verifications/hello")),
                pattern,
                kind: Kind::Api,
                name: Some("hello".into()),
                force: false,
            },
            || panic!("must not prompt"),
        )
        .unwrap()
    }

    #[test]
    fn cargo_and_npm_scaffolds_load_and_validate() {
        for (file, content, expected) in [
            ("Cargo.toml", "[workspace]\n", "cargo"),
            ("package.json", r#"{"scripts":{"test":"vitest"}}"#, "npm"),
        ] {
            for pattern in [Pattern::A, Pattern::B] {
                let tmp = tempfile::tempdir().unwrap();
                fs::write(tmp.path().join(file), content).unwrap();
                let outcome = scaffold(tmp.path(), pattern);
                assert!(outcome.warnings.iter().any(|text| text.contains("review")));
                let loaded = duhem_schema::load(&tmp.path().join("duhem.yml")).unwrap();
                let duhem_schema::Loaded::Manifest {
                    manifest, leaves, ..
                } = loaded
                else {
                    panic!("expected manifest")
                };
                duhem_schema::native::validate_native(&manifest.native).unwrap();
                assert_eq!(manifest.native[0].command[0], expected);
                assert_eq!(leaves.len(), 1);
                duhem_schema::validate(&leaves[0].definition).unwrap();
            }
        }
    }

    #[test]
    fn nextest_requires_documented_junit_configuration() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[workspace]").unwrap();
        fs::create_dir(tmp.path().join(".config")).unwrap();
        for (config, has_report) in [
            ("", false),
            ("[profile.ci.junit]\npath = 'other.xml'", false),
            ("[profile.ci.junit]\npath = 'junit.xml'", true),
        ] {
            fs::write(tmp.path().join(".config/nextest.toml"), config).unwrap();
            let runs = detect_at(tmp.path());
            assert_eq!(runs[0].report.is_some(), has_report);
            assert_eq!(
                runs[0].command[1],
                if has_report { "nextest" } else { "test" }
            );
        }
    }

    #[test]
    fn package_managers_pytest_and_go() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("package.json"),
            r#"{"scripts":{"test":"test"}}"#,
        )
        .unwrap();
        for (lock, manager) in [("yarn.lock", "yarn"), ("pnpm-lock.yaml", "pnpm")] {
            fs::write(tmp.path().join(lock), "").unwrap();
            assert_eq!(detect_at(tmp.path())[0].command[0], manager);
        }
        fs::write(tmp.path().join("go.mod"), "module example").unwrap();
        for marker in ["pytest.ini", "conftest.py", "pyproject.toml"] {
            fs::write(
                tmp.path().join(marker),
                if marker == "pyproject.toml" {
                    "[tool.pytest.ini_options]"
                } else {
                    ""
                },
            )
            .unwrap();
            let runs = detect_at(tmp.path());
            assert_eq!(runs.len(), 3);
            assert_eq!(runs[1].report.as_ref().unwrap().path, "duhem-pytest.xml");
            assert_eq!(runs[2].command, ["go", "test", "./..."]);
            fs::remove_file(tmp.path().join(marker)).unwrap();
        }
    }

    #[test]
    fn existing_manifest_is_preserved_and_suggestions_are_visible() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[workspace]").unwrap();
        let existing = "manifest_version: 1\nverifications: []\n";
        fs::write(tmp.path().join("duhem.yml"), existing).unwrap();
        let outcome = scaffold(tmp.path(), Pattern::B);
        assert_eq!(
            fs::read_to_string(tmp.path().join("duhem.yml")).unwrap(),
            existing
        );
        assert!(outcome.warnings[0].contains("rust-unit"));
        assert!(outcome.warnings[0].contains("native:"));
    }

    #[test]
    fn repository_boundary_and_absent_test_script_do_not_detect() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[workspace]").unwrap();
        let nested = tmp.path().join("nested");
        fs::create_dir_all(nested.join(".git")).unwrap();
        fs::write(
            nested.join("package.json"),
            r#"{"scripts":{"lint":"lint"}}"#,
        )
        .unwrap();
        assert!(detect(&nested).is_none());
    }

    #[test]
    fn no_detection_keeps_templates_byte_identical() {
        for pattern in [Pattern::A, Pattern::B] {
            let tmp = tempfile::tempdir().unwrap();
            scaffold(tmp.path(), pattern);
            let leaf =
                fs::read_to_string(tmp.path().join("verifications/hello/duhem.yml")).unwrap();
            let template = match pattern {
                Pattern::A => include_str!("../templates/init-pattern-a/duhem.api.yml"),
                Pattern::B => include_str!("../templates/init-pattern-b/duhem.api.yml"),
            };
            let expected = template.replace("{{NAME}}", "hello").replace("{{PATH}}", &tmp.path().join("verifications/hello").to_string_lossy())
                .replace("{{SCHEMA}}", "https://raw.githubusercontent.com/onsager-ai/duhem/main/schema/duhem.schema.json");
            assert_eq!(leaf, expected);
            if pattern == Pattern::B {
                assert_eq!(
                    fs::read_to_string(tmp.path().join("verifications/duhem.yml")).unwrap(),
                    include_str!("../templates/init-pattern-b/manifest-stub.yml")
                        .replace("{{LEAF}}", "hello")
                );
            } else {
                assert!(!tmp.path().join("duhem.yml").exists());
            }
        }
    }
}
