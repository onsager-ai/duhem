use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_duhem"))
}

fn invoke(command: &str, path: &Path) -> Output {
    Command::new(bin())
        .arg(command)
        .arg(path)
        .args((command == "run").then_some("--dry-run"))
        .output()
        .expect("spawn duhem")
}

fn malformed_fixture(dir: &Path) -> PathBuf {
    let path = dir.join("repro.yml");
    std::fs::write(
        &path,
        r#"verification: pages subfield repro
pages:
  chat:
    history_item: { css: ':nth-match([data-testid="app-sidebar"] > div, {})' }
criteria:
  - id: AC-1
    description: repro
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/click
            with:
              locator:
                css: $runtime.format($pages.chat.history_item.css, 1)
              timeout: 20s
"#,
    )
    .unwrap();
    path
}

#[test]
fn run_and_validate_agree_that_pages_subfields_are_malformed() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".git")).unwrap();
    let path = malformed_fixture(tmp.path());

    for command in ["run", "validate"] {
        let output = invoke(command, &path);
        assert!(!output.status.success(), "{command} unexpectedly succeeded");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(":14:")
                && stderr.contains("malformed `$pages` reference")
                && stderr.contains("expected `$pages.<page>.<element>`"),
            "{command} emitted the wrong diagnostic: {stderr}"
        );
        assert!(
            !stderr.contains("does not define locally"),
            "{command} misclassified the reference as unresolved: {stderr}"
        );
    }
}

#[test]
fn two_segment_missing_element_remains_unresolved_with_suggestion() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("missing.yml");
    std::fs::write(
        &path,
        r#"verification: missing element
pages:
  chat:
    history_item: { css: '[data-testid="history-item"]' }
criteria:
  - id: AC-1
    description: repro
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/click
            with:
              locator: $pages.chat.history_ite
"#,
    )
    .unwrap();

    let output = invoke("validate", &path);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("references unknown page locator `$pages.chat.history_ite`")
            && stderr.contains("did you mean `history_item`?"),
        "missing element classification changed: {stderr}"
    );
    assert!(!stderr.contains("malformed `$pages` reference"), "{stderr}");
}

#[test]
fn validate_rejects_parameterized_page_arity_at_the_call_site() {
    for (reference, supplied) in [
        ("$pages.chat.history_item", "supplies 0 arguments"),
        ("$pages.chat.history_item(1, 2)", "supplies 2 arguments"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("arity.yml");
        std::fs::write(
            &path,
            format!(
                r#"verification: page arity
pages:
  chat:
    history_item: {{ xpath: '(//article)[{{}}]' }}
criteria:
  - id: AC-1
    description: repro
    checks:
      - id: AC-1.1
        steps:
          - id: select_history
            uses: ui/assert-element
            with:
              locator: {reference}
              expected: visible
"#
            ),
        )
        .unwrap();

        let output = invoke("validate", &path);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(":14:"),
            "missing call-site location: {stderr}"
        );
        assert!(stderr.contains("step `select_history`"), "{stderr}");
        assert!(stderr.contains("$pages.chat.history_item"), "{stderr}");
        assert!(stderr.contains(supplied), "{stderr}");
        assert!(stderr.contains("contains 1 `{}` placeholder"), "{stderr}");
    }
}
