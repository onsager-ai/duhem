//! Reusable flow validation and static expansion (spec #367).
//!
//! Expansion lives in the schema loader so the runtime continues to
//! execute one flat sequence of ordinary catalog actions. The authored
//! `flows:` catalog remains on the definition for round-tripping and
//! dashboard snapshot lookup; only check `steps:` are expanded.

use std::collections::{BTreeMap, BTreeSet};

use crate::assertion::Assertion;
use crate::includes::MAX_INCLUDE_DEPTH;
use crate::step::{ExpandedFlowOrigin, Step};
use crate::verification::{Flow, FlowCatalog, InputDecl, InputType, VerificationDefinition};

pub(crate) const MAX_FLOW_DEPTH: usize = MAX_INCLUDE_DEPTH;

/// Validate every flow-facing rule and expand all check invocations.
pub(crate) fn validate_and_expand(definition: &mut VerificationDefinition) -> Result<(), String> {
    let errors = validate_authored(definition);
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }

    let catalog = definition.flows.clone();
    for criterion in &mut definition.criteria {
        for check in &mut criterion.checks {
            let authored = std::mem::take(&mut check.steps);
            let mut counter = 0usize;
            let expanded = expand_sequence(authored, &catalog, "", None, None, &[], &mut counter);
            check.steps = expanded.steps;
            rewrite_steps(&mut check.steps, &expanded.projections);
            for assertion in &mut check.assertions {
                rewrite_assertion(assertion, &expanded.projections);
            }
            if let Some(session) = &mut check.session {
                *session = rewrite_string(session, &expanded.projections);
            }
        }
    }
    Ok(())
}

/// Validate the authored surface without mutating it. Messages are
/// intentionally self-contained so the public validator and loader can
/// report the same offline diagnostics.
pub(crate) fn validate_authored(definition: &VerificationDefinition) -> Vec<String> {
    let mut errors = Vec::new();

    for (index, step) in definition.setup.iter().enumerate() {
        validate_dispatch(step, &format!("setup step {index}"), &mut errors);
        if let Some(name) = &step.call {
            errors.push(format!(
                "flow `{name}` is invoked from setup step {index}; `call:` is only valid in a check"
            ));
        }
    }

    for (index, step) in definition.teardown.iter().enumerate() {
        validate_dispatch(step, &format!("teardown step {index}"), &mut errors);
        if let Some(name) = &step.call {
            errors.push(format!(
                "flow `{name}` is invoked from teardown step {index}; `call:` is only valid in a check"
            ));
        }
    }

    for (name, flow) in &definition.flows {
        validate_flow(name, flow, &definition.flows, &mut errors);
    }
    validate_flow_graph(&definition.flows, &mut errors);

    for criterion in &definition.criteria {
        for check in &criterion.checks {
            let authored_ids: BTreeSet<&str> = check
                .steps
                .iter()
                .filter_map(|step| step.id.as_deref())
                .collect();
            let mut check_reference = |raw: &str| {
                let Ok(expression) = crate::expr::parse(raw) else {
                    return;
                };
                expression.walk_paths(|path| {
                    if path.root == crate::PathRoot::Steps
                        && let Some(step_id) = path.segments().first()
                        && !authored_ids.contains(step_id.as_str())
                    {
                        errors.push(format!(
                            "criterion `{}` / check `{}`: `{raw}` references undeclared authored step `{step_id}`; inner flow step ids are not caller-addressable",
                            criterion.id, check.id
                        ));
                    }
                });
            };
            for assertion in &check.assertions {
                assertion.walk_exprs(|expression| check_reference(&expression.raw));
            }
            for step in &check.steps {
                walk_strings(&step.with, &mut |raw| {
                    if raw.trim_start().starts_with('$') {
                        check_reference(raw);
                    }
                });
            }
            for (index, step) in check.steps.iter().enumerate() {
                let site = format!(
                    "criterion `{}` / check `{}` / step {index}",
                    criterion.id, check.id
                );
                validate_dispatch(step, &site, &mut errors);
                if let Some(name) = &step.call {
                    validate_call(
                        name,
                        step,
                        &definition.flows,
                        &definition.inputs,
                        None,
                        &site,
                        &mut errors,
                    );
                }
            }
        }
    }

    errors
}

fn validate_dispatch(step: &Step, site: &str, errors: &mut Vec<String>) {
    match (step.uses.as_deref(), step.call.as_deref()) {
        (Some(uses), None) if !uses.trim().is_empty() => {}
        (None, Some(call)) if !call.trim().is_empty() => {}
        (Some(_), Some(_)) => errors.push(format!(
            "{site}: step must declare exactly one of `uses:` or `call:`, not both"
        )),
        _ => errors.push(format!(
            "{site}: step must declare exactly one of `uses:` or `call:`"
        )),
    }
}

fn validate_flow(name: &str, flow: &Flow, catalog: &FlowCatalog, errors: &mut Vec<String>) {
    if flow.steps.is_empty() {
        errors.push(format!("flow `{name}` has no steps"));
    }
    for (param, decl) in &flow.params {
        if decl.kind.is_none() {
            errors.push(format!("flow `{name}` param `{param}` is missing `type:`"));
        }
        if decl.inherit || decl.default.is_some() || decl.env.is_some() {
            errors.push(format!(
                "flow `{name}` param `{param}` may declare only `type:` and `secret:`"
            ));
        }
    }

    let mut ids = BTreeSet::new();
    for (index, step) in flow.steps.iter().enumerate() {
        let site = format!("flow `{name}` step {index}");
        validate_dispatch(step, &site, errors);
        if let Some(id) = &step.id
            && !ids.insert(id.as_str())
        {
            errors.push(format!("flow `{name}` has duplicate step id `{id}`"));
        }
        walk_strings(&step.with, &mut |raw| {
            if raw.contains("$inputs.") || raw.contains("$steps.") {
                errors.push(format!(
                    "flow `{name}` hygiene violation: `{raw}` may reference only `$params.*` and `$pages.*`"
                ));
            }
            if let Some(param) = raw.strip_prefix("$params.").and_then(reference_head)
                && !flow.params.contains_key(param)
            {
                errors.push(format!(
                    "flow `{name}` references unknown param `{param}` in `{raw}`"
                ));
            }
        });
        if let Some(called) = &step.call {
            validate_call(
                called,
                step,
                catalog,
                &BTreeMap::new(),
                Some(&flow.params),
                &site,
                errors,
            );
        }
    }

    for (output, raw) in &flow.outputs {
        let Some((step_id, _)) = step_output_reference(raw) else {
            errors.push(format!(
                "flow `{name}` output `{output}` must reference `$steps.<id>.outputs.<name>`"
            ));
            continue;
        };
        if !ids.contains(step_id)
            && !flow
                .steps
                .iter()
                .any(|step| step.call.is_some() && step.id.as_deref() == Some(step_id))
        {
            errors.push(format!(
                "flow `{name}` output `{output}` references unknown inner step `{step_id}`"
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_call(
    name: &str,
    step: &Step,
    catalog: &FlowCatalog,
    leaf_inputs: &BTreeMap<String, InputDecl>,
    outer_params: Option<&BTreeMap<String, InputDecl>>,
    site: &str,
    errors: &mut Vec<String>,
) {
    let Some(flow) = catalog.get(name) else {
        errors.push(format!("{site}: unknown flow `{name}`"));
        return;
    };
    if flow.params.is_empty() && step.with.is_null() {
        return;
    }
    let Some(with) = step.with.as_mapping() else {
        errors.push(format!(
            "{site}: flow `{name}` expects `with:` to be a parameter map"
        ));
        return;
    };
    let supplied: BTreeMap<&str, &serde_yml::Value> = with
        .iter()
        .filter_map(|(key, value)| key.as_str().map(|key| (key, value)))
        .collect();
    for param in flow.params.keys() {
        if !supplied.contains_key(param.as_str()) {
            errors.push(format!(
                "{site}: flow `{name}` is missing parameter `{param}`"
            ));
        }
    }
    for param in supplied.keys() {
        if !flow.params.contains_key(*param) {
            errors.push(format!(
                "{site}: flow `{name}` received unknown parameter `{param}`"
            ));
        }
    }
    for (param, value) in supplied {
        let Some(expected) = flow.params.get(param).and_then(|decl| decl.kind) else {
            continue;
        };
        if let Some(actual) = value_type(value, leaf_inputs, outer_params)
            && actual != expected
            && !matches!((expected, actual), (InputType::Number, InputType::Integer))
        {
            errors.push(format!(
                "{site}: flow `{name}` parameter `{param}` expects `{expected}`, got `{actual}`"
            ));
        }
    }
}

fn value_type(
    value: &serde_yml::Value,
    leaf_inputs: &BTreeMap<String, InputDecl>,
    outer_params: Option<&BTreeMap<String, InputDecl>>,
) -> Option<InputType> {
    use serde_yml::Value;
    match value {
        Value::String(raw) => {
            if let Some(name) = raw.strip_prefix("$inputs.").and_then(reference_head) {
                return leaf_inputs.get(name).and_then(|decl| decl.kind);
            }
            if let Some(name) = raw.strip_prefix("$params.").and_then(reference_head) {
                return outer_params
                    .and_then(|params| params.get(name))
                    .and_then(|decl| decl.kind);
            }
            if raw.starts_with("$pages.") {
                return Some(InputType::Object);
            }
            if raw.starts_with('$') {
                return None;
            }
            Some(InputType::String)
        }
        Value::Bool(_) => Some(InputType::Boolean),
        Value::Number(number) if number.is_i64() => Some(InputType::Integer),
        Value::Number(_) => Some(InputType::Number),
        Value::Sequence(_) => Some(InputType::Array),
        Value::Mapping(_) => Some(InputType::Object),
        Value::Null | Value::Tagged(_) => None,
    }
}

fn validate_flow_graph(catalog: &FlowCatalog, errors: &mut Vec<String>) {
    fn visit(
        name: &str,
        catalog: &FlowCatalog,
        chain: &mut Vec<String>,
        depth: usize,
        errors: &mut Vec<String>,
    ) {
        if depth > MAX_FLOW_DEPTH {
            errors.push(format!(
                "flow `{name}` exceeds the maximum flow depth of {MAX_FLOW_DEPTH}"
            ));
            return;
        }
        if let Some(index) = chain.iter().position(|item| item == name) {
            let mut cycle = chain[index..].to_vec();
            cycle.push(name.to_string());
            errors.push(format!(
                "flow `{name}` forms a cycle ({})",
                cycle.join(" -> ")
            ));
            return;
        }
        let Some(flow) = catalog.get(name) else {
            return;
        };
        chain.push(name.to_string());
        for called in flow.steps.iter().filter_map(|step| step.call.as_deref()) {
            visit(called, catalog, chain, depth + 1, errors);
        }
        chain.pop();
    }

    for name in catalog.keys() {
        visit(name, catalog, &mut Vec::new(), 0, errors);
    }
}

#[derive(Default)]
struct Expansion {
    steps: Vec<Step>,
    projections: BTreeMap<String, String>,
}

fn expand_sequence(
    steps: Vec<Step>,
    catalog: &FlowCatalog,
    namespace: &str,
    current_flow: Option<&str>,
    current_invocation: Option<&str>,
    inherited_secrets: &[serde_yml::Value],
    counter: &mut usize,
) -> Expansion {
    let mut result = Expansion::default();
    for (inner_index, mut step) in steps.into_iter().enumerate() {
        rewrite_value(&mut step.with, &result.projections);
        if step.call.is_none() {
            if !namespace.is_empty()
                && let Some(id) = &mut step.id
            {
                *id = format!("{namespace}__{id}");
            }
            if let (Some(name), Some(invocation)) = (current_flow, current_invocation) {
                step.flow = Some(ExpandedFlowOrigin {
                    name: name.to_string(),
                    invocation: invocation.to_string(),
                    inner_index: inner_index as u32,
                });
            }
            step.flow_secrets = inherited_secrets.to_vec();
            result.steps.push(step);
            continue;
        }

        let flow_name = step.call.as_deref().expect("validated call");
        let flow = catalog.get(flow_name).expect("validated flow");
        let ordinal = *counter;
        *counter += 1;
        let local_invocation = step
            .id
            .clone()
            .unwrap_or_else(|| format!("{flow_name}#{ordinal}"));
        let invocation = if namespace.is_empty() {
            local_invocation.clone()
        } else {
            format!("{namespace}__{local_invocation}")
        };
        let bindings = mapping_to_bindings(&step.with);
        let mut secrets = inherited_secrets.to_vec();
        for (name, decl) in &flow.params {
            if decl.secret
                && let Some(value) = bindings.get(name)
            {
                secrets.push(value.clone());
            }
        }

        let mut body = flow.steps.clone();
        for inner in &mut body {
            substitute_params(&mut inner.with, &bindings);
        }
        let mut expanded = expand_sequence(
            body,
            catalog,
            &invocation,
            Some(flow_name),
            Some(&invocation),
            &secrets,
            counter,
        );
        for inner in &mut expanded.steps {
            inner.condition = step.condition.clone();
        }

        let mut direct_ids = BTreeMap::new();
        for inner in &flow.steps {
            if inner.call.is_none()
                && let Some(id) = &inner.id
            {
                direct_ids.insert(
                    format!("$steps.{id}."),
                    format!("$steps.{invocation}__{id}."),
                );
            }
        }
        for (output, raw) in &flow.outputs {
            let projected =
                rewrite_string(&rewrite_string(raw, &expanded.projections), &direct_ids);
            if let Some(id) = &step.id {
                result
                    .projections
                    .insert(format!("$steps.{id}.outputs.{output}"), projected);
            }
        }
        result.steps.append(&mut expanded.steps);
    }
    result
}

fn mapping_to_bindings(value: &serde_yml::Value) -> BTreeMap<String, serde_yml::Value> {
    value
        .as_mapping()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| key.as_str().map(|key| (key.to_string(), value.clone())))
        .collect()
}

fn substitute_params(value: &mut serde_yml::Value, bindings: &BTreeMap<String, serde_yml::Value>) {
    match value {
        serde_yml::Value::String(raw) => {
            let Some(reference) = raw.strip_prefix("$params.") else {
                return;
            };
            let mut segments = reference.split('.');
            let Some(name) = segments.next() else {
                return;
            };
            let Some(mut resolved) = bindings.get(name).cloned() else {
                return;
            };
            for segment in segments {
                let Some(next) = resolved
                    .as_mapping()
                    .and_then(|map| map.get(serde_yml::Value::String(segment.to_string())))
                    .cloned()
                else {
                    return;
                };
                resolved = next;
            }
            *value = resolved;
        }
        serde_yml::Value::Sequence(values) => {
            for value in values {
                substitute_params(value, bindings);
            }
        }
        serde_yml::Value::Mapping(values) => {
            for value in values.values_mut() {
                substitute_params(value, bindings);
            }
        }
        _ => {}
    }
}

fn rewrite_steps(steps: &mut [Step], projections: &BTreeMap<String, String>) {
    for step in steps {
        rewrite_value(&mut step.with, projections);
    }
}

fn rewrite_value(value: &mut serde_yml::Value, projections: &BTreeMap<String, String>) {
    match value {
        serde_yml::Value::String(raw) => *raw = rewrite_string(raw, projections),
        serde_yml::Value::Sequence(values) => {
            for value in values {
                rewrite_value(value, projections);
            }
        }
        serde_yml::Value::Mapping(values) => {
            for value in values.values_mut() {
                rewrite_value(value, projections);
            }
        }
        _ => {}
    }
}

fn rewrite_assertion(assertion: &mut Assertion, projections: &BTreeMap<String, String>) {
    let mut value = serde_yml::to_value(&*assertion).expect("assertion serializes");
    rewrite_value(&mut value, projections);
    *assertion = serde_yml::from_value(value).expect("rewritten flow output remains an expression");
}

fn rewrite_string(raw: &str, projections: &BTreeMap<String, String>) -> String {
    let mut entries: Vec<_> = projections.iter().collect();
    entries.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
    entries
        .into_iter()
        .fold(raw.to_string(), |value, (from, to)| value.replace(from, to))
}

fn walk_strings<F: FnMut(&str)>(value: &serde_yml::Value, visit: &mut F) {
    match value {
        serde_yml::Value::String(raw) => visit(raw),
        serde_yml::Value::Sequence(values) => {
            for value in values {
                walk_strings(value, visit);
            }
        }
        serde_yml::Value::Mapping(values) => {
            for value in values.values() {
                walk_strings(value, visit);
            }
        }
        _ => {}
    }
}

fn reference_head(reference: &str) -> Option<&str> {
    reference
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .next()
        .filter(|name| !name.is_empty())
}

fn step_output_reference(raw: &str) -> Option<(&str, &str)> {
    let reference = raw.strip_prefix("$steps.")?;
    let (step, output) = reference.split_once(".outputs.")?;
    (!step.is_empty() && !output.is_empty()).then_some((step, output))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored(yaml: &str) -> VerificationDefinition {
        VerificationDefinition::from_yaml_str(yaml).expect("parse")
    }

    #[test]
    fn expands_params_namespaces_ids_and_projects_outputs() {
        let mut definition = authored(
            r#"
verification: flow
inputs:
  user: { type: string }
flows:
  greet:
    params:
      name: { type: string }
    steps:
      - id: say
        uses: cli/invoke
        with: { command: printf, args: [$params.name] }
    outputs:
      text: $steps.say.outputs.stdout
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - id: first
            call: greet
            with: { name: $inputs.user }
          - id: second
            call: greet
            with: { name: Ada }
        assertions:
          - $steps.first.outputs.text == $steps.second.outputs.text
"#,
        );
        crate::validate(&definition).expect("authored flow validates before expansion");
        validate_and_expand(&mut definition).expect("expand");
        let check = &definition.criteria[0].checks[0];
        assert_eq!(check.steps.len(), 2);
        assert_eq!(check.steps[0].id.as_deref(), Some("first__say"));
        assert_eq!(check.steps[1].id.as_deref(), Some("second__say"));
        assert_eq!(
            check.steps[0].with["args"][0].as_str(),
            Some("$inputs.user")
        );
        assert_eq!(
            check.assertions[0].display(),
            "$steps.first__say.outputs.stdout == $steps.second__say.outputs.stdout"
        );
        assert_eq!(
            check.steps[0].flow.as_ref().map(|flow| flow.name.as_str()),
            Some("greet")
        );
    }

    #[test]
    fn rejects_hygiene_unknown_type_cycle_and_depth_offline() {
        let hygiene = authored(
            r#"
verification: flow
flows:
  bad:
    steps:
      - uses: cli/invoke
        with: { command: $inputs.command }
criteria: []
"#,
        );
        assert!(
            validate_authored(&hygiene)
                .join("\n")
                .contains("flow `bad` hygiene")
        );

        let mismatch = authored(
            r#"
verification: flow
flows:
  typed:
    params:
      count: { type: integer }
    steps:
      - uses: cli/invoke
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - call: typed
            with: { count: nope }
"#,
        );
        assert!(
            validate_authored(&mismatch)
                .join("\n")
                .contains("expects `integer`, got `string`")
        );

        let cycle = authored(
            r#"
verification: flow
flows:
  a:
    steps: [{ call: b }]
  b:
    steps: [{ call: a }]
criteria: []
"#,
        );
        assert!(
            validate_authored(&cycle)
                .join("\n")
                .contains("forms a cycle")
        );

        let unknown = authored(
            r#"
verification: flow
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps: [{ call: missing }]
"#,
        );
        assert!(
            validate_authored(&unknown)
                .join("\n")
                .contains("unknown flow `missing`")
        );

        let over_depth = authored(
            r#"
verification: flow
flows:
  a: { steps: [{ call: b }] }
  b: { steps: [{ call: c }] }
  c: { steps: [{ call: d }] }
  d: { steps: [{ call: e }] }
  e: { steps: [{ uses: cli/invoke }] }
criteria: []
"#,
        );
        let depth_errors = validate_authored(&over_depth).join("\n");
        assert!(
            depth_errors.contains("flow `e` exceeds the maximum flow depth"),
            "{depth_errors}"
        );
    }

    #[test]
    fn rejects_both_or_neither_dispatch_and_caller_access_to_inner_ids() {
        let invalid_dispatch = authored(
            r#"
verification: flow
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: cli/invoke
            call: nope
          - with: {}
"#,
        );
        let errors = crate::validate(&invalid_dispatch).unwrap_err();
        let rendered = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("not both"), "{rendered}");
        assert!(rendered.contains("exactly one"), "{rendered}");

        let inner_reference = authored(
            r#"
verification: flow
flows:
  greet:
    steps:
      - id: say
        uses: cli/invoke
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - id: first
            call: greet
        assertions:
          - exists: $steps.first__say.outputs.stdout
"#,
        );
        let errors = validate_authored(&inner_reference).join("\n");
        assert!(
            errors.contains("inner flow step ids are not caller-addressable"),
            "{errors}"
        );
    }

    #[test]
    fn default_does_not_bypass_caller_step_addressability() {
        let definition = authored(
            r#"
verification: flow
flows:
  greet:
    steps:
      - uses: cli/invoke
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - id: first
            call: greet
        assertions:
          - $runtime.default($steps.bogus.outputs.x, "fb") == "fb"
"#,
        );
        let errors = validate_authored(&definition).join("\n");
        assert!(
            errors.contains("references undeclared authored step `bogus`"),
            "{errors}"
        );
    }

    #[test]
    fn invocation_condition_applies_to_every_expanded_step() {
        let mut definition = authored(
            r#"
verification: flow
flows:
  cleanup:
    steps:
      - uses: cli/invoke
      - uses: cli/invoke
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - call: cleanup
            if: always
"#,
        );
        validate_and_expand(&mut definition).expect("expand");
        assert!(
            definition.criteria[0].checks[0]
                .steps
                .iter()
                .all(|step| step.condition == crate::StepCondition::Always)
        );
    }

    #[test]
    fn nested_flow_provenance_names_the_immediate_namespaced_invocation() {
        let mut definition = authored(
            r#"
verification: nested flow
flows:
  child:
    steps:
      - id: action
        uses: cli/invoke
  parent:
    steps:
      - id: nested
        call: child
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - id: outer
            call: parent
"#,
        );
        validate_and_expand(&mut definition).expect("expand");
        let step = &definition.criteria[0].checks[0].steps[0];
        assert_eq!(step.id.as_deref(), Some("outer__nested__action"));
        assert_eq!(
            step.flow.as_ref().map(|flow| (
                flow.name.as_str(),
                flow.invocation.as_str(),
                flow.inner_index
            )),
            Some(("child", "outer__nested", 0))
        );
    }
}
