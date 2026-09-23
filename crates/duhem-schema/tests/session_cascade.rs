use duhem_schema::{VerificationDefinition, validate};

fn parse(yaml: &str) -> VerificationDefinition {
    VerificationDefinition::from_yaml_str(yaml).unwrap()
}

#[test]
fn session_null_and_absent_round_trip_at_all_scalar_sites() {
    let yaml = "verification: x\nsession: ~\ncriteria:\n- id: AC-1\n  description: x\n  session: ~\n  checks:\n  - id: AC-1.1\n    session: ~\n    assertions: ['true']\n";
    let vd = parse(yaml);
    assert_eq!(vd.session, Some(None));
    assert_eq!(vd.criteria[0].session, Some(None));
    assert_eq!(vd.criteria[0].checks[0].session, Some(None));
    assert_eq!(parse(&vd.to_yaml_string().unwrap()), vd);
    let absent = parse(
        &yaml
            .lines()
            .filter(|line| !line.trim_start().starts_with("session:"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert!(absent.session.is_none());
}

#[test]
fn enclosing_seeds_validate_and_own_setup_seeds_have_locations() {
    let yaml = r#"
verification: x
inputs:
  state: { type: object }
session: $inputs.state
setup:
  - id: leaf
    uses: ui/capture-session
    outputs: { state: state }
criteria:
  - id: AC-1
    description: x
    session: $setup.leaf.outputs.state
    setup:
      - id: criterion
        uses: ui/capture-session
        outputs: { state: state }
    checks:
      - id: AC-1.1
        session: $setup.criterion.outputs.state
        setup:
          - id: own
            uses: ui/capture-session
            outputs: { state: state }
        assertions: ['true']
"#;
    validate(&parse(yaml)).unwrap();
    for bad in [
        yaml.replace(
            "session: $inputs.state",
            "session: $setup.leaf.outputs.state",
        ),
        yaml.replace(
            "session: $setup.leaf.outputs.state",
            "session: $setup.criterion.outputs.state",
        ),
        yaml.replace(
            "session: $setup.criterion.outputs.state",
            "session: $setup.own.outputs.state",
        ),
    ] {
        let errors = validate(&parse(&bad)).unwrap_err();
        assert!(
            errors.iter().any(|error| error.location().is_some()),
            "{errors:?}"
        );
    }
}

#[test]
fn call_selector_is_named_only_and_fills_flow_browser_steps() {
    let yaml = r#"
verification: x
flows:
  inner:
    steps:
      - uses: ui/navigate
        with: { url: 'about:blank' }
      - uses: ui/navigate
        session: other
        with: { url: 'about:blank' }
  outer:
    steps:
      - call: inner
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        sessions: { actor: ~, other: ~ }
        steps:
          - call: outer
            session: actor
        assertions: ['true']
"#;
    validate(&parse(yaml)).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("verification.yml");
    std::fs::write(&path, yaml).unwrap();
    let loaded = duhem_schema::load(&path).unwrap();
    let duhem_schema::Loaded::Leaf { definition, .. } = loaded else {
        panic!("leaf")
    };
    let steps = &definition.criteria[0].checks[0].steps;
    assert_eq!(steps[0].session.as_deref(), Some("actor"));
    assert_eq!(steps[1].session.as_deref(), Some("other"));
    for declaration in ["", "        session: ~\n"] {
        let invalid = yaml.replace("        sessions: { actor: ~, other: ~ }\n", declaration);
        let errors = validate(&parse(&invalid)).unwrap_err();
        assert!(errors.iter().any(|error| error.location().is_some()));
    }
}

#[test]
fn loader_rejects_selector_even_when_flow_has_no_browser_steps() {
    let yaml = "verification: x\nflows:\n  task:\n    steps: [{uses: cli/invoke}]\ncriteria:\n- id: AC-1\n  description: x\n  checks:\n  - id: AC-1.1\n    steps: [{call: task, session: actor}]\n    assertions: ['true']\n";
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("verification.yml");
    std::fs::write(&path, yaml).unwrap();
    let error = duhem_schema::load(&path).unwrap_err().to_string();
    assert!(
        error.contains("named-sessions check") && error.contains("line"),
        "{error}"
    );
}

#[test]
fn enclosing_flow_outputs_project_into_scalar_and_named_seeds() {
    let yaml = r#"
verification: Flow-acquired seeds
flows:
  acquire:
    steps:
      - id: capture
        uses: ui/capture-session
        outputs: { state: state }
    outputs: { state: $steps.capture.outputs.state }
setup:
  - id: leaf_login
    call: acquire
criteria:
  - id: AC-1
    description: Seeds can consume enclosing flow outputs.
    session: $setup.leaf_login.outputs.state
    setup:
      - id: criterion_login
        call: acquire
    checks:
      - id: AC-1.1
        session: $setup.criterion_login.outputs.state
        assertions: ['true']
      - id: AC-1.2
        sessions: { actor: $setup.criterion_login.outputs.state }
        assertions: ['true']
"#;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("verification.yml");
    std::fs::write(&path, yaml).unwrap();
    let duhem_schema::Loaded::Leaf { definition, .. } = duhem_schema::load(&path).unwrap() else {
        panic!("leaf")
    };
    assert_eq!(
        definition.criteria[0].session,
        Some(Some("$setup.leaf_login__capture.outputs.state".into()))
    );
    assert_eq!(
        definition.criteria[0].checks[0].session,
        Some(Some("$setup.criterion_login__capture.outputs.state".into()))
    );
    assert_eq!(
        definition.criteria[0].checks[1].sessions.as_ref().unwrap()["actor"]
            .as_ref()
            .unwrap()
            .raw,
        "$setup.criterion_login__capture.outputs.state"
    );
    validate(&definition).unwrap();
}

#[test]
fn call_selector_reaches_browser_actions_inside_a_flow_loop() {
    let yaml = r#"
verification: Flow loop selector
inputs:
  rows: { type: array, default: [1, 2] }
flows:
  browse:
    params:
      rows: { type: array }
    steps:
      - for_each: $params.rows
        max: 2
        steps:
          - uses: ui/navigate
            with: { url: 'about:blank' }
criteria:
  - id: AC-1
    description: Loop wrappers propagate selectors without having an action name.
    checks:
      - id: AC-1.1
        sessions: { actor: ~ }
        steps:
          - call: browse
            session: actor
            with: { rows: $inputs.rows }
        assertions: ['true']
"#;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("verification.yml");
    std::fs::write(&path, yaml).unwrap();
    let duhem_schema::Loaded::Leaf { definition, .. } = duhem_schema::load(&path).unwrap() else {
        panic!("leaf")
    };
    assert_eq!(
        definition.criteria[0].checks[0].steps[0].for_each_body[0]
            .session
            .as_deref(),
        Some("actor")
    );
    validate(&definition).unwrap();
}
