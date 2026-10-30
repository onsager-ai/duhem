//! Shared enumeration for rules that apply to every authored step block.

use crate::source::SourcePathSegment as Segment;
use crate::{Check, SourceLocation, Step, VerificationDefinition};

/// A step-bearing block and its diagnostic context. Only check bodies carry
/// `check`; lifecycle blocks and reusable templates never contribute judgment.
pub struct StepBlock<'a> {
    pub steps: &'a [Step],
    pub site: String,
    pub check: Option<&'a Check>,
    pub(crate) path: Vec<Segment>,
}

impl StepBlock<'_> {
    /// Human-readable identity of a step within this block.
    pub fn step_site(&self, index: usize) -> String {
        let separator = if self.check.is_some() { " /" } else { "" };
        format!("{}{separator} step {index}", self.site)
    }

    /// Authored location of a step field, when retained by the loader.
    pub fn location(
        &self,
        definition: &VerificationDefinition,
        index: usize,
        field: &str,
    ) -> Option<SourceLocation> {
        let mut path = self.path.clone();
        path.extend([Segment::index(index), Segment::key(field)]);
        if let [
            Segment::Key(_),
            Segment::Index(criterion),
            Segment::Key(_),
            Segment::Index(check),
            Segment::Key(body),
        ] = self.path.as_slice()
            && body == "steps"
            && field == "with"
        {
            return definition
                .source_map
                .check_step_with_location(*criterion, *check, index);
        }
        definition.source_map.node_location(&path)
    }
}

/// All step collections, including lifecycle scopes, flow templates and nested
/// loop bodies. Rules visit each block's direct steps rather than re-enumerating
/// containers or recursively calling `Step::actions` themselves.
pub fn step_blocks(definition: &VerificationDefinition) -> Vec<StepBlock<'_>> {
    let mut blocks = Vec::new();
    for (phase, steps) in [
        ("setup", &definition.setup),
        ("teardown", &definition.teardown),
    ] {
        collect(
            steps,
            phase.into(),
            vec![Segment::key(phase)],
            None,
            &mut blocks,
        );
    }
    for (name, fixture) in &definition.fixtures {
        for (phase, steps) in [("up", &fixture.up), ("down", &fixture.down)] {
            collect(
                steps,
                format!("fixture `{name}` {phase}"),
                vec![
                    Segment::key("fixtures"),
                    Segment::key(name),
                    Segment::key(phase),
                ],
                None,
                &mut blocks,
            );
        }
    }
    for (name, flow) in &definition.flows {
        collect(
            &flow.steps,
            format!("flow `{name}`"),
            vec![
                Segment::key("flows"),
                Segment::key(name),
                Segment::key("steps"),
            ],
            None,
            &mut blocks,
        );
    }
    for (ci, criterion) in definition.criteria.iter().enumerate() {
        let path = vec![Segment::key("criteria"), Segment::index(ci)];
        let label = format!("criterion `{}`", criterion.id);
        for (phase, steps) in [
            ("setup", &criterion.setup),
            ("teardown", &criterion.teardown),
        ] {
            let mut path = path.clone();
            path.push(Segment::key(phase));
            collect(steps, format!("{label} {phase}"), path, None, &mut blocks);
        }
        for (hi, check) in criterion.checks.iter().enumerate() {
            let label = format!("{label} / check `{}`", check.id);
            for (phase, steps) in [
                ("setup", &check.setup),
                ("steps", &check.steps),
                ("teardown", &check.teardown),
            ] {
                let mut path = path.clone();
                path.extend([
                    Segment::key("checks"),
                    Segment::index(hi),
                    Segment::key(phase),
                ]);
                let body = phase == "steps";
                let site = if body {
                    label.clone()
                } else {
                    format!("{label} {phase}")
                };
                collect(steps, site, path, body.then_some(check), &mut blocks);
            }
        }
    }
    blocks
}

fn collect<'a>(
    steps: &'a [Step],
    site: String,
    path: Vec<Segment>,
    check: Option<&'a Check>,
    blocks: &mut Vec<StepBlock<'a>>,
) {
    for (index, step) in steps.iter().enumerate() {
        let nested = if step.for_each_body.is_empty() {
            step.steps.as_deref().unwrap_or(&[])
        } else {
            &step.for_each_body
        };
        if !nested.is_empty() {
            let mut nested_path = path.clone();
            nested_path.extend([Segment::index(index), Segment::key("steps")]);
            collect(nested, site.clone(), nested_path, check, blocks);
        }
    }
    blocks.push(StepBlock {
        steps,
        site,
        check,
        path,
    });
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn walker_covers_the_schema_step_collections() {
        let definition = VerificationDefinition::from_yaml_str(
            "verification: all blocks\nsetup:\n  - for_each: '$inputs.items'\n    as: item\n    max: 1\n    steps:\n      - uses: api/call\nflows:\n  sample:\n    steps: []\nfixtures:\n  sample:\n    up: []\n    down: []\ncriteria:\n  - id: AC-1\n    description: all blocks\n    checks:\n      - id: AC-1.1\n        steps: []\n",
        ).unwrap();
        let observed: BTreeSet<String> = step_blocks(&definition)
            .iter()
            .map(|block| {
                let (owner, field) = match block.path.as_slice() {
                    [Segment::Key(field)] => ("VerificationDefinition", field.as_str()),
                    [Segment::Key(root), _, Segment::Key(field)] if root == "fixtures" => {
                        ("Fixture", field.as_str())
                    }
                    [Segment::Key(root), _, Segment::Key(field)] if root == "flows" => {
                        ("Flow", field.as_str())
                    }
                    [Segment::Key(root), _, Segment::Key(field)] if root == "criteria" => {
                        ("Criterion", field.as_str())
                    }
                    [
                        Segment::Key(root),
                        _,
                        Segment::Key(checks),
                        _,
                        Segment::Key(field),
                    ] if root == "criteria" && checks == "checks" => ("Check", field.as_str()),
                    _ => ("Step", "steps"),
                };
                format!("{owner}.{field}")
            })
            .collect();
        let schema = crate::json_schema();
        let mut declared = BTreeSet::new();
        for (owner, definition) in schema["definitions"].as_object().unwrap() {
            if let Some(properties) = definition["properties"].as_object() {
                for (field, property) in properties {
                    if property["items"]["$ref"].as_str() == Some("#/definitions/Step") {
                        declared.insert(format!("{owner}.{field}"));
                    }
                }
            }
        }
        assert_eq!(
            observed, declared,
            "new schema step collections must join the shared walker"
        );
    }

    #[test]
    fn expanded_loop_bodies_are_visited_once_with_the_enclosing_scope() {
        let mut definition = VerificationDefinition::from_yaml_str(
            "verification: loop\nsetup:\n  - uses: cli/invoke\ncriteria: []\n",
        )
        .unwrap();
        let body = definition.setup[0].clone();
        definition.setup[0].uses = None;
        definition.setup[0].for_each_body = vec![body];
        let sites: Vec<_> = step_blocks(&definition)
            .into_iter()
            .flat_map(|block| {
                block
                    .steps
                    .iter()
                    .filter_map(|step| step.uses.as_ref().map(|_| block.site.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(sites, ["setup"]);
    }
}
