//! Flow replacements for the former fixture value/scope checks (#618).
use std::process::Command;

#[test]
fn flow_body_and_cleanup_validate_conditions_loops_and_output_scope() {
    for phase in ["steps", "cleanup"] {
        for field in ["if", "for_each"] {
            for output in ["stdout", "stdoutt"] {
                let extra = if field == "for_each" {
                    "        max: 5\n"
                } else {
                    ""
                };
                let step = format!(
                    "      - uses: cli/invoke\n        {field}: $steps.rows.outputs.{output}\n{extra}"
                );
                let (body, cleanup) = if phase == "steps" {
                    (
                        format!("      - id: rows\n        uses: cli/invoke\n{step}"),
                        "      - uses: cli/invoke\n".to_string(),
                    )
                } else {
                    (
                        "      - id: rows\n        uses: cli/invoke\n".to_string(),
                        step,
                    )
                };
                let yaml = format!(
                    "verification: flow values\nflows:\n  res:\n    steps:\n{body}    cleanup:\n{cleanup}criteria:\n  - id: AC\n    description: Flow values\n    checks:\n      - id: c\n        setup: [{{call: res}}]\n        assertions: [\"true\"]\n"
                );
                let tmp = tempfile::tempdir().unwrap();
                let path = tmp.path().join("flow.yml");
                std::fs::write(&path, yaml).unwrap();
                let output_result = Command::new(env!("CARGO_BIN_EXE_duhem"))
                    .arg("validate")
                    .arg(&path)
                    .output()
                    .unwrap();
                let stderr = String::from_utf8_lossy(&output_result.stderr);
                assert_eq!(
                    output_result.status.success(),
                    output == "stdout",
                    "{phase} {field}: {stderr}"
                );
                if output == "stdoutt" {
                    assert!(stderr.contains("stdoutt"), "{stderr}");
                }
            }
        }
    }
}
