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
                    ("check", "assertions" | "session" | "sessions")
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
                visit(item, path, source, context)?;
                path.pop();
            }
        }
        Value::String(raw) if context == "expression" => {
            if let Ok(expression) = crate::expr::parse(raw) {
                let mut removed = false;
                expression.walk_paths(|path| {
                    removed |= path.root == PathRoot::Loop
                        && path
                            .segments()
                            .first()
                            .is_some_and(|name| name == "fixture");
                });
                if removed {
                    return Err(removed_fixture_error(
                        "$fixture",
                        source.node_location(path),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
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
