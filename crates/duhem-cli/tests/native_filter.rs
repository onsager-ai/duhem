//! Native filter selection runs real child processes and excludes the rest.
#![cfg(unix)]
use std::process::Command;

fn run(
    path: &std::path::Path,
    tmp: &std::path::Path,
    filters: &[&str],
    reporter: &str,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_duhem"));
    command
        .args(["run", "--no-live", "--reporter", reporter, "--db"])
        .arg(tmp.join("evidence.db"))
        .arg(path);
    for filter in filters {
        command.args(["--filter", filter]);
    }
    command.output().unwrap()
}

#[test]
fn native_filter_selects_only_matching_ids_and_excludes_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("leaf.yml");
    std::fs::write(
        &leaf,
        r#"verification: native-filter
native:
  - {id: alpha, command: [sh, -c, 'touch alpha']}
  - {id: beta, command: [sh, -c, 'touch beta']}
criteria:
  - id: AC
    description: independent
    checks:
      - id: body
        assertions: ['true']
"#,
    )
    .unwrap();
    let output = run(&leaf, tmp.path(), &["native:a*"], "json");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !output.status.success(),
        "empty criterion aggregation remains inconclusive"
    );
    assert!(tmp.path().join("alpha").exists());
    assert!(!tmp.path().join("beta").exists());
    assert_eq!(summary["claim_scope"]["checks"]["selected"], 0);
    assert_eq!(summary["native"].as_array().unwrap().len(), 1);
    assert_eq!(summary["native"][0]["id"], "alpha");
    assert_eq!(
        summary["claim_scope"]["excluded_native_runs"],
        serde_json::json!(["beta"])
    );
}

#[test]
fn criterion_filter_skips_native_and_reports_all_as_excluded() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("leaf.yml");
    std::fs::write(
        &leaf,
        r#"verification: native-filter
native:
  - {id: alpha, command: [sh, -c, 'touch alpha']}
criteria:
  - id: AC
    description: independent
    checks:
      - id: body
        assertions: ['true']
"#,
    )
    .unwrap();
    let output = run(&leaf, tmp.path(), &["AC"], "json");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{summary}");
    assert!(!tmp.path().join("alpha").exists());
    assert!(summary["native"].as_array().unwrap().is_empty());
    assert_eq!(
        summary["claim_scope"]["excluded_native_runs"],
        serde_json::json!(["alpha"])
    );
    let text = run(&leaf, tmp.path(), &["AC"], "default");
    assert!(String::from_utf8_lossy(&text.stdout).contains("excluded native runs: alpha"));
}

#[test]
fn repeated_native_filters_are_or_combined() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("leaf.yml");
    std::fs::write(
        &leaf,
        r#"verification: native-filter
native:
  - {id: alpha, command: [sh, -c, 'touch alpha']}
  - {id: beta, command: [sh, -c, 'touch beta']}
  - {id: gamma, command: [sh, -c, 'touch gamma']}
criteria: []
"#,
    )
    .unwrap();
    let output = run(&leaf, tmp.path(), &["native:alpha", "native:beta"], "json");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{summary}");
    assert!(tmp.path().join("alpha").exists());
    assert!(tmp.path().join("beta").exists());
    assert!(!tmp.path().join("gamma").exists());
    assert_eq!(summary["native"].as_array().unwrap().len(), 2);
    assert_eq!(
        summary["claim_scope"]["excluded_native_runs"],
        serde_json::json!(["gamma"])
    );
}
