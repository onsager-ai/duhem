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
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(output.status.success(), "{summary}");
    assert_eq!(summary["verdict"], "pass");
    assert!(summary.get("body_verdict").is_none());
    assert!(tmp.path().join("alpha").exists());
    assert!(!tmp.path().join("beta").exists());
    assert_eq!(summary["claim_scope"]["checks"]["selected"], 0);
    assert_eq!(summary["claim_scope"]["criteria"]["selected"], 0);
    assert_eq!(
        summary["claim_scope"]["excluded"].as_array().unwrap().len(),
        1
    );
    assert_eq!(summary["native"].as_array().unwrap().len(), 1);
    assert_eq!(summary["native"][0]["id"], "alpha");
    assert_eq!(
        summary["claim_scope"]["excluded_native_runs"],
        serde_json::json!(["beta"])
    );

    let failing = tmp.path().join("failing.yml");
    std::fs::write(
        &failing,
        r#"verification: native-filter
native:
  - {id: alpha, command: [sh, -c, 'exit 7']}
criteria:
  - id: AC
    description: independent
    checks:
      - id: body
        assertions: ['true']
"#,
    )
    .unwrap();
    let output = run(&failing, tmp.path(), &["native:alpha"], "json");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(!output.status.success(), "{summary}");
    assert_eq!(summary["verdict"], "fail");
    assert!(summary.get("body_verdict").is_none());
}

#[test]
fn unmatched_native_only_filter_keeps_empty_selection_inconclusive() {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("leaf.yml");
    std::fs::write(
        &leaf,
        "verification: unmatched\nnative: [{id: alpha, command: [sh, -c, 'touch alpha']}]\ncriteria:\n  - id: AC\n    description: independent\n    checks: [{id: body, assertions: ['true']}]\n",
    )
    .unwrap();
    let output = run(&leaf, tmp.path(), &["native:missing"], "json");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(!output.status.success(), "{summary}");
    assert_eq!(summary["verdict"], "inconclusive:empty_aggregation");
    assert!(summary.get("native").is_none(), "{summary}");
    assert_eq!(summary["claim_scope"]["checks"]["selected"], 0);
    assert_eq!(
        summary["claim_scope"]["excluded_native_runs"],
        serde_json::json!(["alpha"])
    );
}

#[test]
fn native_only_filter_passes_manifest_with_vacuous_leaf_bodies() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("alpha.yml"),
        "verification: alpha\ncriteria:\n  - id: AC\n    description: excluded\n    checks: [{id: body, assertions: ['true']}]\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("beta.yml"),
        "verification: beta\ncriteria:\n  - id: AC\n    description: excluded\n    checks: [{id: body, assertions: ['true']}]\n",
    )
    .unwrap();
    let up = tmp.path().join("up.sh");
    std::fs::write(&up, "#!/bin/sh\nexit 0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&up, std::fs::Permissions::from_mode(0o755)).unwrap();
    let manifest = tmp.path().join("duhem.yml");
    std::fs::write(
        &manifest,
        "manifest_version: 1\nprovision: {up: ./up.sh}\nnative:\n  - {id: suite, command: [sh, -c, 'touch suite']}\n  - {id: other, command: [sh, -c, 'touch other']}\nverifications: [{path: alpha.yml}, {path: beta.yml}]\n",
    )
    .unwrap();
    let output = run(&manifest, tmp.path(), &["native:suite"], "json");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(output.status.success(), "{summary}");
    assert_eq!(summary["verdict"], "pass");
    assert!(tmp.path().join("suite").exists());
    assert!(!tmp.path().join("other").exists());
    assert_eq!(
        summary["selection"]["excluded_native_runs"],
        serde_json::json!(["other"])
    );
    assert_eq!(summary["runs"].as_array().unwrap().len(), 2);
    for run in summary["runs"].as_array().unwrap() {
        assert_eq!(run["claim_scope"]["checks"]["selected"], 0);
        assert_eq!(run["verdict"], "pass");
        assert!(run.get("body_verdict").is_none());
    }
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
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(output.status.success(), "{summary}");
    assert!(!tmp.path().join("alpha").exists());
    assert!(summary.get("native").is_none(), "{summary}");
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
criteria:
  - id: AC
    description: independent
    checks:
      - id: body
        assertions: ['true']
"#,
    )
    .unwrap();
    let output = run(&leaf, tmp.path(), &["native:alpha", "native:beta"], "json");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
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
