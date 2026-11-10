const COVERAGE_YAML: &str = include_str!("coverage-blocks.yaml");

fn coverage_cases() -> Vec<(String, String)> {
    let definition = duhem_schema::VerificationDefinition::from_yaml_str(COVERAGE_YAML).unwrap();
    duhem_schema::step_walk::step_blocks(&definition)
        .iter()
        .map(|block| {
            let step = block
                .steps
                .iter()
                .find(|step| step.uses.is_some())
                .expect("every walker block must have a fixture action");
            (
                block.site.clone(),
                step.id.clone().expect("fixture action needs an id"),
            )
        })
        .collect()
}

fn coverage_definition(
    id: &str,
    action: &str,
    fields: &[&str],
) -> duhem_schema::VerificationDefinition {
    let lines: Vec<_> = COVERAGE_YAML.lines().collect();
    let mut yaml = String::new();
    let mut found = false;
    for (index, line) in lines.iter().enumerate() {
        if index > 0 && lines[index - 1].trim() == format!("- id: {id}") {
            assert_eq!(line.trim(), "uses: api/call");
            let indent = &line[..line.len() - line.trim_start().len()];
            yaml.push_str(&format!("{indent}uses: {action}\n"));
            for field in fields {
                yaml.push_str(&format!("{indent}{field}\n"));
            }
            found = true;
        } else {
            yaml.push_str(line);
            yaml.push('\n');
        }
    }
    assert!(found, "fixture cannot place defect in {id}");
    duhem_schema::VerificationDefinition::from_yaml_str(&yaml).unwrap()
}
