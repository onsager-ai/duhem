use std::process::Command;

#[test]
fn failing_iteration_is_named_in_default_and_plugin_json_detail_without_new_fields() {
    let temp = tempfile::tempdir().unwrap();
    let leaf = temp.path().join("case.yml");
    std::fs::write(
        &leaf,
        r#"
verification: loop report
inputs:
  rows: { type: array, default: [1, 0, 3] }
criteria:
  - id: AC
    description: Every row has a positive value
    checks:
      - id: C
        steps:
          - for_each: $inputs.rows
            as: row
            max: 3
            id: query
            uses: db/query
            with: { connection: 'sqlite::memory:', sql: 'select 1 where ? > 0', params: [$row] }
        assertions: ['$steps.query.outputs.row_count == 1']
"#,
    )
    .unwrap();
    for reporter in ["default", "json"] {
        let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
            .arg("run")
            .arg(&leaf)
            .arg("--db")
            .arg(temp.path().join(format!("{reporter}.db")))
            .args(["--no-live", "--reporter", reporter])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.contains("for_each step 1, iteration 2 of 3"),
            "stdout={text} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(text.contains("actual 0, expected 1"), "{text}");
        if reporter == "json" {
            let report: serde_json::Value = serde_json::from_str(&text).unwrap();
            let assertion = &report["failures"][0]["assertions"][0];
            assert!(assertion.get("iteration").is_none());
            assert!(assertion.get("iteration_count").is_none());
            assert_eq!(report["schema_version"], "2");
        }
    }
}
