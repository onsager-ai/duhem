//! Black-box manifest input coverage (spec #354).
//!
//! Unit tests pin precedence and opt-in typing. These tests drive the
//! shipped CLI through the real resolver, runtime, and SQLite evidence
//! sink so a shared secret inherited by multiple leaves cannot regress
//! to plaintext at either the run header or step boundary.

use std::path::{Path, PathBuf};
use std::process::Command;

use duhem_evidence::{EventPayload, SqliteStore, Store};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_duhem"))
}

fn write(dir: &Path, name: &str, contents: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn leaf(name: &str) -> String {
    format!(
        r#"
verification: {name}
inputs:
  password: {{ inherit: true, secret: true }}
criteria:
  - id: AC-1
    description: The inherited credential reaches the real command.
    checks:
      - id: AC-1.1
        steps:
          - id: echo
            uses: cli/invoke
            with:
              command: ["printf", "%s", $inputs.password]
        assertions:
          - $steps.echo.outputs.stdout == $inputs.password
"#
    )
}

#[tokio::test]
async fn inherited_manifest_secret_is_masked_in_both_leaves_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let secret = "crawlab-fixture-password-354";
    let db = tmp.path().join("duhem.db");
    write(tmp.path(), "a/duhem.yml", &leaf("first consumer"));
    write(tmp.path(), "b/duhem.yml", &leaf("second consumer"));
    write(
        tmp.path(),
        "duhem.yml",
        r#"
manifest_version: 1
inputs:
  password:
    type: string
    env: CRAWLAB_PASSWORD
    secret: true
verifications:
  - path: a/duhem.yml
  - path: b/duhem.yml
"#,
    );

    let output = Command::new(bin())
        .arg("run")
        .arg(tmp.path())
        .arg("--db")
        .arg(&db)
        .env("CRAWLAB_PASSWORD", secret)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "run failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));

    let store = SqliteStore::open_read_only(&db).await.unwrap();
    let runs = store.list_runs().await.unwrap();
    assert_eq!(runs.len(), 3, "manifest parent plus two inheriting leaves");
    let parent = runs
        .iter()
        .find(|run| run.lineage.parent_run_id.is_none())
        .expect("manifest parent");
    let leaves: Vec<_> = runs
        .iter()
        .filter(|run| run.lineage.parent_run_id.as_deref() == Some(parent.run_id.as_str()))
        .collect();
    assert_eq!(leaves.len(), 2, "one suite child per inheriting leaf");
    for run in leaves {
        assert_eq!(
            run.inputs["password"],
            serde_json::json!("[redacted:password]"),
            "run header is masked for {}",
            run.verification
        );
        let events = store.run_events(&run.run_id).await.unwrap();
        let started = events
            .iter()
            .find_map(|event| match &event.payload {
                EventPayload::StepStarted { with, .. } => Some(with),
                _ => None,
            })
            .expect("step_started recorded");
        assert!(
            serde_json::to_string(started)
                .unwrap()
                .contains("[redacted:password]"),
            "step with-map is masked for {}: {started:?}",
            run.verification
        );
        assert!(
            events
                .iter()
                .all(|event| !serde_json::to_string(event).unwrap().contains(secret)),
            "no event payload retains plaintext for {}",
            run.verification
        );
    }
}

#[test]
fn unused_manifest_declaration_warns_and_dry_run_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "leaf.yml",
        r#"
verification: no inherited inputs
criteria:
  - id: AC-1
    description: The suite remains valid while its declarations evolve.
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#,
    );
    write(
        tmp.path(),
        "duhem.yml",
        r#"
manifest_version: 1
inputs:
  future_token: { type: string, env: FUTURE_TOKEN, secret: true }
verifications:
  - path: leaf.yml
"#,
    );

    let output = Command::new(bin())
        .arg("run")
        .arg(tmp.path())
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("WOULD RUN"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("warning:")
            && stderr.contains("future_token")
            && stderr.contains("not inherited"),
        "warning is surfaced without failing the run: {stderr}"
    );
}

#[test]
fn manifest_secret_default_fails_cli_validation() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "leaf.yml",
        &leaf("invalid declaration consumer"),
    );
    write(
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

    let output = Command::new(bin())
        .arg("validate")
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("password")
            && stderr.contains("secret: true")
            && stderr.contains("default:"),
        "CLI exposes the manifest validation rule: {stderr}"
    );
}

#[test]
fn manifest_without_declarations_keeps_existing_fixture_output_exactly() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verifications/inherits-example");
    let output = Command::new(bin())
        .arg("run")
        .arg(fixture)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "WOULD RUN: reads-inherited::AC-1::AC-1.1\n\
         RESOLVED INPUT: reads-inherited::base_url = https://inherits.example.com\n\
         RESOLVED INPUT: reads-inherited::region = us-west\n"
    );
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "profile: default\n"
    );
}

/// Spec #368 collapsed `inherits:` into `inputs:` but deliberately kept
/// the opt-in rather than letting a leaf read manifest inputs freely.
/// The property that justified keeping it: a `$inputs.<name>` the leaf
/// never declared stays a hard validation error *even when the parent
/// manifest declares that same name*. Without it a typo resolves
/// silently against an unrelated manifest input and the check runs green
/// against the wrong target — the one failure mode a verification tool
/// cannot have. Guarded here because `validate()` alone sees a single
/// document and cannot express "the manifest also declares this".
#[test]
fn leaf_ref_to_an_undeclared_name_fails_even_when_the_manifest_declares_it() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "leaf.yml",
        r#"
verification: reads a name it never declared
criteria:
  - id: AC-1
    description: The leaf's declaration surface stays closed.
    checks:
      - id: AC-1.1
        steps:
          - id: echo
            uses: cli/invoke
            with:
              command: ["printf", "%s", $inputs.api_url]
        assertions:
          - $steps.echo.outputs.stdout == $inputs.api_url
"#,
    );
    write(
        tmp.path(),
        "duhem.yml",
        r#"
manifest_version: 1
inputs:
  api_url: { type: string, default: http://127.0.0.1:8080 }
verifications:
  - path: leaf.yml
"#,
    );

    let output = Command::new(bin())
        .arg("validate")
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a manifest-declared name must not satisfy an undeclared leaf reference"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("undeclared input `api_url`"),
        "fails for the right reason, not an unrelated error: {stderr}"
    );
}
