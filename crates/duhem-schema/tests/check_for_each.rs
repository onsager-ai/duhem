use duhem_schema::VerificationDefinition;

fn load(yaml: &str) -> VerificationDefinition {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("verification.yml");
    std::fs::write(&path, yaml).unwrap();
    let duhem_schema::Loaded::Leaf { definition, .. } = duhem_schema::load(&path).unwrap() else {
        panic!("expected leaf")
    };
    definition
}

const VD: &str = r#"
verification: rows
inputs: {rows: {type: array}}
criteria:
  - id: AC-1
    description: Rows validate
    checks:
      - id: AC-1.1
        steps:
          - for_each: $inputs.rows
            max: 5
            as: row
            steps:
              - id: inspect
                uses: cli/invoke
                with: {command: [echo, $row]}
                outputs: {text: stdout}
        assertions: ['$steps.inspect.outputs.text == "ok"']
"#;

#[test]
fn bounds_outputs_and_binding_scope_are_checked() {
    duhem_schema::validate(&VerificationDefinition::from_yaml_str(VD).unwrap()).unwrap();
    let valid = load(VD);
    duhem_schema::validate(&valid).unwrap();
    assert_eq!(
        valid.criteria[0].checks[0].worst_case_step_count().unwrap(),
        5
    );
    for (bad, expected) in [
        (VD.replace("            max: 5\n", ""), "requires `max:`"),
        (VD.replace("$inputs.rows", "$inputs.typo"), "typo"),
        (
            VD.replace("[echo, $row]", "[echo, $typo]"),
            "binding in scope",
        ),
        (VD.replace("outputs.text ==", "outputs.typo =="), "typo"),
        (
            VD.replace(
                "assertions: ['$steps.inspect.outputs.text == \"ok\"']",
                "assertions: ['$row == 1']",
            ),
            "binding in scope",
        ),
    ] {
        let bad = load(&bad);
        let errors = duhem_schema::validate(&bad).unwrap_err();
        assert!(
            errors.iter().any(|e| e.to_string().contains(expected)),
            "{errors:?}"
        );
    }
}

#[test]
fn flow_calls_cannot_hide_nested_loops() {
    let yaml = VD.replace("inputs:", "flows:\n  nested:\n    steps:\n      - for_each: $params.rows\n        max: 2\n        uses: cli/invoke\n    params: {rows: {type: array}}\ninputs:")
        .replace("              - id: inspect\n                uses: cli/invoke\n                with: {command: [echo, $row]}\n                outputs: {text: stdout}", "              - call: nested\n                with: {rows: $inputs.rows}")
        .replace("assertions: ['$steps.inspect.outputs.text == \"ok\"']", "assertions: ['true']");
    let def = load(&yaml);
    let errors = duhem_schema::validate(&def).unwrap_err();
    assert!(
        errors.iter().any(|e| e.to_string().contains("depth")),
        "{errors:?}"
    );
}

#[test]
fn even_an_unused_flow_must_bound_its_loop() {
    let yaml = VD.replace("inputs:", "flows:\n  unused:\n    steps:\n      - for_each: $params.rows\n        uses: cli/invoke\n    params: {rows: {type: array}}\ninputs:");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("verification.yml");
    std::fs::write(&path, yaml).unwrap();
    assert!(
        duhem_schema::load(&path)
            .unwrap_err()
            .to_string()
            .contains("requires `max:`")
    );
}
