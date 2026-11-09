//! Deprecated `fixtures:` / `needs:` desugaring (#618).
//!
//! A fixture is a flow with `cleanup:` that is called at the end of the
//! consuming check's `setup:` (#615):
//!
//! | Deprecated form | Desugared form |
//! |---|---|
//! | `fixtures.<name>: { up, down, cleanup_policy }` | a flow with `steps: up`, `cleanup: down`, the same policy |
//! | `needs: [a, b]` on a check | `call:` of `a`, `b`, in order, at the **end** of that check's `setup:` |
//! | `$fixture.<name>.<id>.outputs.*` in `down:` | `$steps.<id>.outputs.*`, the body reference of the same call |
//!
//! The expansion mirrors the loader's flow expansion: the `up:` steps are
//! appended to the check's `setup:`, each carrying the cleanup the call
//! registers. The cleanup records the fixture name, so lifecycle evidence
//! keeps the fixture and the consuming check. The authored definition is not
//! changed: validation, `duhem describe` and the hook chain keep reading
//! `fixtures:` / `needs:`, and the engine desugars at the start of a run.

use std::borrow::Cow;

use crate::expr::ExprStr;
use crate::source::SourcePathSegment;
use crate::step::{FlowCleanup, Step, StepCondition};
use crate::verification::{Fixture, VerificationDefinition};

/// Desugar every `needs:` fixture onto its check's `setup:`. Borrows the
/// definition unchanged when no check needs a declared fixture.
pub fn desugar_fixtures(definition: &VerificationDefinition) -> Cow<'_, VerificationDefinition> {
    let consumed = definition
        .criteria
        .iter()
        .flat_map(|criterion| &criterion.checks)
        .any(|check| {
            check
                .needs
                .iter()
                .any(|name| definition.fixtures.contains_key(name))
        });
    if !consumed {
        return Cow::Borrowed(definition);
    }
    let mut desugared = definition.clone();
    for criterion in &mut desugared.criteria {
        for check in &mut criterion.checks {
            for (index, name) in check.needs.iter().enumerate() {
                let Some(fixture) = definition.fixtures.get(name) else {
                    continue;
                };
                check
                    .setup
                    .extend(call_steps(name, &format!("{name}#{index}"), fixture));
            }
        }
    }
    Cow::Owned(desugared)
}

/// The `up:` steps of one `needs:` entry. Every step carries the cleanup the
/// call registers; `call` is unique per entry, so a fixture listed twice is
/// two instances with two cleanups.
fn call_steps(name: &str, call: &str, fixture: &Fixture) -> Vec<Step> {
    let reference = format!("$fixture.{name}.");
    let mut cleanup = fixture.down.clone();
    for step in &mut cleanup {
        rewrite_step(step, &reference);
    }
    let cleanup = FlowCleanup {
        flow: name.to_string(),
        call: call.to_string(),
        policy: fixture.cleanup_policy,
        steps: cleanup,
        aliases: Vec::new(),
        per_iteration: false,
        fixture: Some(name.to_string()),
    };
    let mut steps = fixture.up.clone();
    if steps.is_empty() {
        steps.push(empty_up());
    }
    for step in &mut steps {
        step.flow_cleanup.insert(0, cleanup.clone());
    }
    steps
}

/// Stands for a fixture whose `up:` has no steps (validation rejects it, but
/// the cleanup is still registered): a loop with no body, which dispatches
/// nothing and is dropped before the block runs.
fn empty_up() -> Step {
    serde_yml::from_str("{ for_each: 'true', max: 0 }").expect("a bodiless loop step parses")
}

/// Whether a step is the placeholder [`empty_up`] stands in with.
pub fn is_empty_up(step: &Step) -> bool {
    step.for_each.is_some() && step.for_each_body.is_empty()
}

/// `$fixture.<name>.` becomes `$steps.`: the fixture's step outputs are the
/// body outputs of the call whose cleanup this is.
fn rewrite_step(step: &mut Step, reference: &str) {
    rewrite_value(&mut step.with, reference);
    if let StepCondition::Expr(condition) = &mut step.condition {
        *condition = rewrite_expr(condition, reference);
    }
    if let Some(source) = &mut step.for_each {
        *source = rewrite_expr(source, reference);
    }
    for inner in step
        .for_each_body
        .iter_mut()
        .chain(step.steps.iter_mut().flatten())
    {
        rewrite_step(inner, reference);
    }
}

fn rewrite_expr(expr: &ExprStr, reference: &str) -> ExprStr {
    ExprStr::from_source(&expr.raw.replace(reference, "$steps."))
        .expect("rewriting a valid fixture reference preserves expression syntax")
}

fn rewrite_value(value: &mut serde_yml::Value, reference: &str) {
    match value {
        serde_yml::Value::String(raw) => *raw = raw.replace(reference, "$steps."),
        serde_yml::Value::Sequence(values) => {
            for value in values {
                rewrite_value(value, reference);
            }
        }
        serde_yml::Value::Mapping(values) => {
            for value in values.values_mut() {
                rewrite_value(value, reference);
            }
        }
        _ => {}
    }
}

/// One deprecation warning per declared fixture, with its source location.
/// Empty for a definition that declares none.
pub fn fixture_deprecation_warnings(definition: &VerificationDefinition) -> Vec<String> {
    definition
        .fixtures
        .keys()
        .map(|name| {
            let location = definition
                .source_map
                .node_location(&[
                    SourcePathSegment::key("fixtures"),
                    SourcePathSegment::key(name),
                ])
                .map(|at| format!("{}:{}: ", at.line, at.column))
                .unwrap_or_default();
            format!(
                "{location}`fixtures.{name}` is deprecated and will be removed in v0.9.0. Declare `flows.{name}` with `steps:` (was `up:`), `cleanup:` (was `down:`) and the same `cleanup_policy:`; inside `cleanup:` replace `$fixture.{name}.<id>.outputs.*` with `$steps.<id>.outputs.*`, and elsewhere declare it in the flow's `outputs:` and read `$setup.<call-id>.outputs.<name>`; replace `needs: [{name}]` with `- id: {name}` + `call: {name}` at the end of the check's `setup:`."
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The one deprecated-form definition kept in test code (#618): pins the
    // desugaring and the warning.
    const DEPRECATED: &str = r#"
verification: deprecated fixtures
fixtures:
  project:
    cleanup_policy: diagnostic_only
    up:
      - id: create
        uses: api/call
        with: { url: /projects }
    down:
      - uses: api/call
        with: { url: "$fixture.project.create.outputs.body.url" }
        if: "$fixture.project.create.outputs.status == 201"
  extra:
    up: [{ uses: cli/invoke }]
    down: [{ uses: cli/invoke }]
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        setup: [{ id: own, uses: cli/invoke }]
        needs: [project, extra, project]
        steps: [{ uses: cli/invoke }]
        assertions: ["true"]
"#;

    fn parsed() -> VerificationDefinition {
        VerificationDefinition::from_yaml_str(DEPRECATED).expect("parse")
    }

    #[test]
    fn needs_become_calls_at_the_end_of_check_setup() {
        let authored = parsed();
        let desugared = desugar_fixtures(&authored);
        let check = &desugared.criteria[0].checks[0];
        let order: Vec<_> = check
            .setup
            .iter()
            .map(|step| {
                step.flow_cleanup
                    .first()
                    .map_or("authored".to_string(), |c| c.call.clone())
            })
            .collect();
        assert_eq!(order, ["authored", "project#0", "extra#1", "project#2"]);
        // The authored definition is untouched.
        assert_eq!(authored.criteria[0].checks[0].setup.len(), 1);
        assert_eq!(authored.criteria[0].checks[0].needs.len(), 3);
    }

    #[test]
    fn cleanup_carries_policy_fixture_and_rewritten_references() {
        let authored = parsed();
        let desugared = desugar_fixtures(&authored);
        let cleanup = &desugared.criteria[0].checks[0].setup[1].flow_cleanup[0];
        assert_eq!(cleanup.fixture.as_deref(), Some("project"));
        assert_eq!(cleanup.flow, "project");
        assert_eq!(
            cleanup.policy,
            crate::verification::CleanupPolicy::DiagnosticOnly
        );
        let down = &cleanup.steps[0];
        assert_eq!(
            down.with["url"].as_str(),
            Some("$steps.create.outputs.body.url")
        );
        let StepCondition::Expr(condition) = &down.condition else {
            panic!("condition kept");
        };
        assert_eq!(condition.raw, "$steps.create.outputs.status == 201");
        let extra = &desugared.criteria[0].checks[0].setup[2].flow_cleanup[0];
        assert!(extra.policy.is_required());
    }

    #[test]
    fn a_definition_without_consumed_fixtures_is_borrowed() {
        let mut authored = parsed();
        authored.criteria[0].checks[0].needs.clear();
        assert!(matches!(desugar_fixtures(&authored), Cow::Borrowed(_)));
    }

    #[test]
    fn one_warning_per_declared_fixture_with_a_location_and_the_rewrite() {
        let warnings = fixture_deprecation_warnings(&parsed());
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        let project = warnings
            .iter()
            .find(|w| w.contains("`fixtures.project`"))
            .expect("project warning");
        assert!(
            project.contains("is deprecated and will be removed in v0.9.0"),
            "{project}"
        );
        assert!(project.contains("`needs: [project]` with `- id: project` + `call: project`"));
        assert!(project.contains("read `$setup.<call-id>.outputs.<name>`"));
        assert!(
            project.chars().next().is_some_and(|c| c.is_ascii_digit()),
            "starts with line:column: {project}"
        );
    }

    #[test]
    fn a_flow_only_definition_warns_nothing() {
        let definition = VerificationDefinition::from_yaml_str(
            "verification: flows\nflows:\n  f:\n    steps: [{ uses: cli/invoke }]\ncriteria: []\n",
        )
        .expect("parse");
        assert!(fixture_deprecation_warnings(&definition).is_empty());
    }

    // The migration the warning names for a check body that read a fixture's
    // output: declare it in the flow's `outputs:`, give the call an `id:`.
    #[test]
    fn a_migrated_check_body_reads_a_cleanup_flows_declared_output() {
        let definition = VerificationDefinition::from_yaml_str(
            r#"
verification: migrated
flows:
  project:
    steps:
      - id: create
        uses: cli/invoke
        with: { command: [echo, made] }
    outputs:
      name: $steps.create.outputs.stdout
    cleanup:
      - uses: cli/invoke
        with: { command: [echo, "$steps.create.outputs.stdout"] }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        setup: [{ id: project, call: project }]
        steps:
          - uses: cli/invoke
            with: { command: [echo, "$setup.project.outputs.name"] }
        assertions: ["true"]
"#,
        )
        .expect("parse");
        crate::validate_with_contract_outputs(&definition, &|_| vec!["stdout".into()])
            .expect("the declared output is readable through the call id");
    }
}
