//! Regression coverage for #538, also runnable against the pre-fix CLI.

use std::process::{Command, Output};

use serde_json::json;

fn validate(scope: &str, phase: &str, step: serde_json::Value) -> Output {
    let mut document = json!({
        "verification": "Lifecycle validation",
        "criteria": [{
            "id": "AC-1", "description": "Lifecycle blocks use the action contract",
            "checks": [{"id": "AC-1.1", "steps": [], "assertions": ["true"]}]
        }]
    });
    let container = if scope == "criterion" {
        &mut document["criteria"][0]
    } else {
        &mut document["criteria"][0]["checks"][0]
    };
    container[phase] = json!([step]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("duhem.yml");
    std::fs::write(&path, serde_yml::to_string(&document).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_duhem"))
        .arg("validate")
        .arg(&path)
        .current_dir(directory.path())
        .output()
        .unwrap()
}

#[test]
fn lifecycle_unknown_actions_fields_enums_outputs_and_secret_paths_are_rejected() {
    let mut failures = Vec::new();
    for scope in ["criterion", "check"] {
        for phase in ["setup", "teardown"] {
            let valid = validate(
                scope,
                phase,
                json!({"uses": "api/call", "with": {"url": "http://example.invalid"}}),
            );
            assert!(
                valid.status.success(),
                "{scope} {phase}: {}",
                String::from_utf8_lossy(&valid.stderr)
            );
            for (step, diagnostic) in [
                (json!({"uses": "missing/action"}), "unknown action"),
                (
                    json!({"uses": "api/call", "with": {"url": "u", "bogus": 1}}),
                    "no `with:` field `bogus`",
                ),
                (
                    json!({"uses": "ui/assert-element", "with": {"locator": {"css": "h1"}, "expected": "shown"}}),
                    "`expected` = `shown`",
                ),
                (
                    json!({"uses": "api/call", "with": {"url": "u"}, "outputs": {"alias": "missing"}}),
                    "produces no output `missing`",
                ),
                (
                    json!({"uses": "api/call", "with": {"url": "u"}, "secret_outputs": ["missing.token"]}),
                    "secret output path",
                ),
            ] {
                let output = validate(scope, phase, step);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if output.status.success() || !stderr.contains(diagnostic) {
                    failures.push(format!("{scope} {phase} missed {diagnostic}: {stderr}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn lifecycle_identity_bindings_warn_without_changing_validation_success() {
    let mut failures = Vec::new();
    for scope in ["criterion", "check"] {
        for phase in ["setup", "teardown"] {
            let output = validate(
                scope,
                phase,
                json!({
                    "uses": "api/call", "with": {"url": "u"},
                    "outputs": {"status": "status"}
                }),
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{scope} {phase}: {stderr}");
            if !stderr.contains("identity binding")
                || !stderr.contains(phase)
                || !stderr.contains("AC-1")
            {
                failures.push(format!(
                    "{scope} {phase} missed identity-binding warning: {stderr}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
