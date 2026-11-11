//! Located migration errors for removed authoring syntax, without inspecting opaque data.
use crate::source::SourcePathSegment as Segment;
use crate::verification::{SchemaError, removed_fixture_error};
use crate::{PathRoot, SourceMap};
use serde_yml::Value;

pub(crate) fn validate(document: &Value, source: &SourceMap) -> Result<(), SchemaError> {
    visit(document, &mut Vec::new(), source, "root")
}

fn visit(
    value: &Value,
    path: &mut Vec<Segment>,
    source: &SourceMap,
    context: &str,
) -> Result<(), SchemaError> {
    if context == "assertion" {
        if let Ok(assertion) = serde_yml::from_value::<crate::Assertion>(value.clone()) {
            let mut failure = None;
            assertion.walk_exprs(|expression| {
                if has_fixture_root(&expression.parsed) && failure.is_none() {
                    let location = match path.as_slice() {
                        [
                            _,
                            Segment::Index(ci),
                            _,
                            Segment::Index(ki),
                            _,
                            Segment::Index(ai),
                        ] => source.assertion_location(*ci, *ki, *ai, &assertion, &expression.raw),
                        _ => None,
                    }
                    .or_else(|| source.node_location(path));
                    failure = Some(removed_fixture_error("$fixture", location));
                }
            });
            return failure.map_or(Ok(()), Err);
        }
        return Ok(());
    }
    match value {
        Value::Mapping(map) => {
            for (key, value) in map {
                let Some(key) = key.as_str() else { continue };
                path.push(Segment::key(key));
                if (context == "root" && key == "fixtures")
                    || (matches!(context, "check" | "step") && key == "needs")
                {
                    return Err(removed_fixture_error(key, source.key_location(path)));
                }
                let next = match (context, key) {
                    ("root", "criteria") => Some("criterion"),
                    ("criterion", "checks") => Some("check"),
                    ("root", "flows") => Some("catalog"),
                    ("catalog", _) => Some("flow"),
                    (
                        "root" | "criterion" | "check" | "flow" | "step",
                        "setup" | "teardown" | "steps" | "cleanup",
                    ) => Some("step"),
                    ("check", "assertions") => Some("assertions"),
                    ("root" | "criterion" | "check", "session")
                    | ("check", "sessions")
                    | ("step", "with" | "if" | "for_each" | "session")
                    | ("flow", "outputs") => Some("expression"),
                    ("expression", _) => Some("expression"),
                    _ => None,
                };
                if let Some(next) = next {
                    visit(value, path, source, next)?;
                }
                path.pop();
            }
        }
        Value::Sequence(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(Segment::index(index));
                visit(
                    item,
                    path,
                    source,
                    if context == "assertions" {
                        "assertion"
                    } else {
                        context
                    },
                )?;
                path.pop();
            }
        }
        Value::String(raw) if context == "expression" => {
            if let Ok(expression) = crate::expr::parse(raw)
                && has_fixture_root(&expression)
            {
                return Err(removed_fixture_error(
                    "$fixture",
                    source.node_location(path),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn has_fixture_root(expression: &crate::Expr) -> bool {
    let mut removed = false;
    expression.walk_paths(|path| {
        removed |= path.root == PathRoot::Loop
            && path
                .segments()
                .first()
                .is_some_and(|name| name == "fixture");
    });
    removed
}

#[cfg(test)]
mod tests {
    use crate::includes::PartialRootManifest;
    use crate::{RootManifest, VerificationDefinition};

    #[test]
    fn opaque_data_prose_and_expression_literals_are_preserved() {
        VerificationDefinition::from_yaml_str(
            r#"verification: x
metadata: {needs: [database], note: '$fixture.thing'}
criteria:
  - id: AC
    description: Read $fixture.thing as prose
    checks:
      - id: c
        assertions: ["'$fixture.thing' == '$fixture.thing'"]
"#,
        )
        .unwrap();
    }

    #[test]
    fn assertion_literal_sets_and_patterns_are_not_expressions() {
        let yaml = r#"verification: literal assertion data
criteria:
  - id: AC
    description: Literal data remains literal
    checks:
      - id: c
        assertions:
          - in: {value: "'$fixture.example'", set: ["$fixture.example"]}
          - matches: {value: "'$fixture.example'", pattern: "$fixture.example"}
          - equal: ["'$fixture.example'", "'$fixture.example'"]
"#;
        VerificationDefinition::from_yaml_str(yaml).unwrap();
        for assertion in [
            "in: {value: $fixture.res.make.outputs.value, set: [literal]}",
            "matches: {value: $fixture.res.make.outputs.value, pattern: literal}",
            "type_check: {value: $fixture.res.make.outputs.value, is: string}",
            "exists: $fixture.res.make.outputs.value",
            r#"equal: ['$fixture.res.make.outputs.value', '"literal"']"#,
        ] {
            let source = format!(
                "verification: x\ncriteria:\n  - id: AC\n    description: x\n    checks:\n      - id: c\n        assertions:\n          - {assertion}\n"
            );
            let error = VerificationDefinition::from_yaml_str(&source).unwrap_err();
            assert!(
                error.to_string().contains("Declare `flows.<name>`"),
                "{error}"
            );
            assert!(error.location().is_some(), "{error}");
        }
    }

    #[test]
    fn duplicate_equal_operands_fall_back_to_the_assertion_location() {
        let yaml = r#"verification: duplicate operands
criteria:
  - id: AC
    description: Duplicate removed references retain a location
    checks:
      - id: c
        assertions:
          - equal: ["$fixture.res.make.outputs.value", "$fixture.res.make.outputs.value"]
"#;
        let error = VerificationDefinition::from_yaml_str(yaml).unwrap_err();
        assert!(
            error.to_string().contains("Declare `flows.<name>`"),
            "{error}"
        );
        assert_eq!(
            error.location(),
            Some(crate::SourceLocation {
                line: 8,
                column: 13
            })
        );
    }

    #[test]
    fn every_session_scope_reports_the_located_migration() {
        let reference = "$fixture.res.make.outputs.state";
        for yaml in [
            format!("verification: x\nsession: {reference}\ncriteria: []\n"),
            format!(
                "verification: x\ncriteria:\n  - id: AC\n    description: x\n    session: {reference}\n    checks: []\n"
            ),
            format!(
                "verification: x\ncriteria:\n  - id: AC\n    description: x\n    checks:\n      - id: c\n        session: {reference}\n        assertions: [true]\n"
            ),
            format!(
                "verification: x\nsetup:\n  - uses: ui/navigate\n    session: {reference}\ncriteria: []\n"
            ),
        ] {
            let error = VerificationDefinition::from_yaml_str(&yaml).unwrap_err();
            assert!(
                error.to_string().contains("Declare `flows.<name>`"),
                "{error}"
            );
            assert!(error.location().is_some(), "{error}");
        }
    }

    #[test]
    fn every_loader_rejects_removed_shared_flow_fields_and_expressions() {
        for field in ["needs: [res]", "if: $fixture.res.make.outputs.ok"] {
            let catalog =
                format!("flows:\n  res:\n    steps:\n      - uses: cli/invoke\n        {field}\n");
            let leaf = format!("verification: x\n{catalog}criteria: []\n");
            let root = format!("manifest_version: 1\n{catalog}verifications: []\n");
            for error in [
                VerificationDefinition::from_yaml_str(&leaf).unwrap_err(),
                RootManifest::from_yaml_str(&root).unwrap_err(),
                PartialRootManifest::from_yaml_str(&catalog).unwrap_err(),
            ] {
                assert!(
                    error.to_string().contains("Declare `flows.<name>`"),
                    "{error}"
                );
                assert!(error.location().is_some(), "{error}");
            }
        }
    }
}
