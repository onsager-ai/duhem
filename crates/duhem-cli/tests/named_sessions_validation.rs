//! Offline diagnostics exercise the real CLI, including root-manifest composition.
use std::process::Command;

fn definition(fields: &str, step: &str) -> String {
    format!(
        "verification: sessions\ninputs:\n  state: {{ type: object }}\ncriteria:\n  - id: AC-1\n    description: x\n    checks:\n      - id: AC-1.1\n{fields}        steps:\n{step}        assertions: [\"true\"]\n"
    )
}

#[test]
fn rejected_shapes_report_file_line_column_before_run() {
    let cases = [
        (
            "        sessions: { admin: ~, user1: ~ }\n",
            "          - uses: ui/navigate\n            session: missing\n",
            "undeclared session",
            "session: missing",
            22,
        ),
        (
            "        sessions: { admin: ~ }\n",
            "          - uses: ui/navigate\n            session: $inputs.state\n",
            "never an expression",
            "session: $inputs",
            22,
        ),
        (
            "        session: $inputs.state\n        sessions: { admin: ~ }\n",
            "          - uses: ui/navigate\n            session: admin\n",
            "mutually exclusive",
            "sessions:",
            19,
        ),
        (
            "        sessions: { admin: ~ }\n",
            "          - uses: api/call\n            session: admin\n",
            "browser-driving",
            "session: admin",
            22,
        ),
        (
            "        sessions: { admin: ~ }\n",
            "          - uses: db/query\n            session: admin\n",
            "browser-driving",
            "session: admin",
            22,
        ),
        (
            "        sessions: { admin: ~ }\n",
            "          - uses: cli/invoke\n            session: admin\n",
            "browser-driving",
            "session: admin",
            22,
        ),
        (
            "        sessions: { admin: ~ }\n",
            "          - uses: ui/navigate\n",
            "must name",
            "uses:",
            19,
        ),
        (
            "",
            "          - uses: ui/navigate\n            session: admin\n",
            "undeclared session",
            "session: admin",
            22,
        ),
        (
            "        sessions: { a: ~, b: ~, c: ~, d: ~, e: ~ }\n",
            "          - uses: ui/navigate\n            session: a\n",
            "defaults.max_sessions (4)",
            "sessions:",
            19,
        ),
        (
            "        sessions: { admin: 'true' }\n",
            "          - uses: ui/navigate\n            session: admin\n",
            "whole-string",
            "sessions:",
            28,
        ),
    ];
    for (fields, step, expected, marker, column) in cases {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("invalid.yml");
        let source = definition(fields, step);
        let line = source
            .lines()
            .position(|line| line.contains(marker))
            .unwrap()
            + 1;
        std::fs::write(&path, source).unwrap();
        for action in ["validate", "run"] {
            let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
                .arg(action)
                .arg(&path)
                .output()
                .unwrap();
            assert!(!output.status.success());
            let message = String::from_utf8(output.stderr).unwrap();
            assert!(message.contains(expected), "{action}: {message}");
            assert!(
                message.contains(&format!("{}:{line}:{column}:", path.display())),
                "{action}: {message}"
            );
        }
    }
}

#[test]
fn root_manifest_ceiling_applies_to_suite_and_direct_leaf() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("duhem.yml");
    let leaf = tmp.path().join("leaf.yml");
    std::fs::write(&leaf, definition("        sessions: { admin: ~, user1: ~ }\n", "          - uses: ui/assert-url\n            session: admin\n            with: { matches: /login }\n")).unwrap();
    for (limit, valid) in [(1, false), (2, true), (8, true)] {
        std::fs::write(&root, format!("manifest_version: 1\ndefaults:\n  max_sessions: {limit}\nverifications:\n  - path: leaf.yml\n")).unwrap();
        for path in [&root, &leaf] {
            let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
                .arg("validate")
                .arg(path)
                .output()
                .unwrap();
            assert_eq!(
                output.status.success(),
                valid,
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            if !valid {
                let message = String::from_utf8_lossy(&output.stderr);
                assert!(
                    message.contains(&format!("{}:9:19:", leaf.display())),
                    "{message}"
                );
                assert!(message.contains("defaults.max_sessions (1)"));
            }
        }
    }
}

#[test]
fn old_definition_round_trips_byte_identically_and_single_session_is_allowed() {
    let old = "verification: old\ncriteria:\n- id: AC-1\n  description: x\n  checks:\n  - id: AC-1.1\n    steps:\n    - uses: ui/assert-url\n      with:\n        matches: /login\n";
    let def = duhem_schema::VerificationDefinition::from_yaml_str(old).unwrap();
    assert_eq!(serde_yml::to_string(&def).unwrap(), old);
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("one.yml");
    std::fs::write(&path, definition("        sessions: { admin: ~ }\n", "          - uses: ui/assert-url\n            session: admin\n            with: { matches: /login }\n")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .arg("validate")
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("one entry"));
}
