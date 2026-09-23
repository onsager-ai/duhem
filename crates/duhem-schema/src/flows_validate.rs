//! Authored flow validation, separate from static expansion.
use super::*;

/// Validate authored flows without mutation, for both loader and validator.
pub(crate) fn validate_authored(definition: &VerificationDefinition) -> Vec<String> {
    let mut errors = Vec::new();

    validate_lifecycle_dispatch(&definition.setup, "setup step", definition, &mut errors);
    validate_lifecycle_dispatch(
        &definition.teardown,
        "teardown step",
        definition,
        &mut errors,
    );
    for (fixture, lifecycle) in &definition.fixtures {
        validate_lifecycle_dispatch(
            &lifecycle.up,
            &format!("fixture `{fixture}` up step"),
            definition,
            &mut errors,
        );
        validate_lifecycle_dispatch(
            &lifecycle.down,
            &format!("fixture `{fixture}` down step"),
            definition,
            &mut errors,
        );
    }
    for criterion in &definition.criteria {
        validate_lifecycle_dispatch(
            &criterion.setup,
            &format!("criterion `{}` setup step", criterion.id),
            definition,
            &mut errors,
        );
        validate_lifecycle_dispatch(
            &criterion.teardown,
            &format!("criterion `{}` teardown step", criterion.id),
            definition,
            &mut errors,
        );
        for check in &criterion.checks {
            validate_lifecycle_dispatch(
                &check.setup,
                &format!(
                    "criterion `{}` / check `{}` setup step",
                    criterion.id, check.id
                ),
                definition,
                &mut errors,
            );
            validate_lifecycle_dispatch(
                &check.teardown,
                &format!(
                    "criterion `{}` / check `{}` teardown step",
                    criterion.id, check.id
                ),
                definition,
                &mut errors,
            );
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
                .flat_map(|step| {
                    std::iter::once(step)
                        .chain(step.steps.iter().flatten())
                        .chain(step.for_each_body.iter())
                })
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
            for (index, step) in check
                .steps
                .iter()
                .flat_map(|s| std::iter::once(s).chain(s.steps.iter().flatten()))
                .enumerate()
            {
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

    errors.extend(crate::session_calls::validate(definition));
    errors
}

/// Validate one lifecycle step list — `setup:`, `teardown:`, fixture
/// `up:`/`down:`, or a criterion-/check-level `setup:`/`teardown:`
/// (§10.3.6). A direct `call:` is legitimate here (#526), same as a
/// `for_each:` step's `call:` body form (#443) — both get the same
/// flow-param validation a check's `call:` step already gets.
fn validate_lifecycle_dispatch(
    steps: &[Step],
    label: &str,
    definition: &VerificationDefinition,
    errors: &mut Vec<String>,
) {
    for (index, step) in steps.iter().enumerate() {
        let site = format!("{label} {index}");
        validate_dispatch(step, &site, errors);
        if let Some(name) = &step.call {
            validate_call(
                name,
                step,
                &definition.flows,
                &definition.inputs,
                None,
                &site,
                errors,
            );
        }
        if step.for_each.is_some()
            && let Some(body) = &step.steps
        {
            // The `steps:` body form (depth capped at 1 by
            // `validate_for_each` — not re-checked here): each body
            // step still needs the ordinary exactly-one-of-`uses:`/
            // `call:` dispatch check, and a `call:` body step gets the
            // same flow-param validation any other `call:` gets.
            for (inner_index, inner) in body.iter().enumerate() {
                let inner_site = format!("{site} body step {inner_index}");
                validate_dispatch(inner, &inner_site, errors);
                if let Some(name) = &inner.call {
                    validate_call(
                        name,
                        inner,
                        &definition.flows,
                        &definition.inputs,
                        None,
                        &inner_site,
                        errors,
                    );
                }
            }
        }
    }
}

fn validate_dispatch(step: &Step, site: &str, errors: &mut Vec<String>) {
    // `for_each:` (#443 Tier 1) extends the exactly-one rule from two
    // body forms to three — `uses:`, `call:`, or an inline `steps:`
    // list — rather than introducing a separate concept. `steps:` and
    // `max:`/`as:` are meaningless without an enclosing `for_each:`,
    // so they're rejected there instead of silently ignored.
    if step.for_each.is_some() {
        let forms = [
            step.uses.as_deref().is_some_and(|u| !u.trim().is_empty()),
            step.call.as_deref().is_some_and(|c| !c.trim().is_empty()),
            step.steps.is_some(),
        ];
        match forms.iter().filter(|present| **present).count() {
            1 => {}
            0 => errors.push(format!(
                "{site}: a `for_each:` step must declare exactly one of `uses:`, `call:`, or `steps:` as its body"
            )),
            _ => errors.push(format!(
                "{site}: a `for_each:` step must declare exactly one of `uses:`, `call:`, or `steps:` as its body, not more than one"
            )),
        }
        return;
    }
    if step.steps.is_some() {
        errors.push(format!(
            "{site}: `steps:` is only valid on a step that also declares `for_each:`"
        ));
        return;
    }
    if step.max.is_some() {
        errors.push(format!(
            "{site}: `max:` is only valid on a step that also declares `for_each:`"
        ));
        return;
    }
    if step.as_binding.is_some() {
        errors.push(format!(
            "{site}: `as:` is only valid on a step that also declares `for_each:`"
        ));
        return;
    }
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
    for (index, step) in flow
        .steps
        .iter()
        .flat_map(|s| std::iter::once(s).chain(s.steps.iter().flatten()))
        .enumerate()
    {
        let site = format!("flow `{name}` step {index}");
        validate_dispatch(step, &site, errors);
        if step.for_each.is_some() {
            if step.max.is_none() {
                errors.push(format!("{site}: `for_each:` requires `max:`"));
            }
            if step.id.is_some() || !step.outputs.is_empty() || !step.secret_outputs.is_empty() {
                errors.push(format!(
                    "{site}: a `for_each:` wrapper may not declare id, outputs, or secret_outputs"
                ));
            }
            if step
                .steps
                .iter()
                .flatten()
                .any(|inner| inner.for_each.is_some() || inner.steps.is_some())
            {
                errors.push(format!("{site}: `for_each:` body exceeds depth 1"));
            }
        }
        // This formerly blocked loops entering judging checks through a flow.
        // Tier 2 now evaluates each expanded iteration; boundedness and depth
        // are still validated after expansion, including indirect flow bodies.
        if let Some(id) = &step.id
            && !ids.insert(id.as_str())
        {
            errors.push(format!("flow `{name}` has duplicate step id `{id}`"));
        }
        let mut references = vec![step.with.clone()];
        if let Some(expr) = &step.for_each {
            references.push(serde_yml::Value::String(expr.raw.clone()));
        }
        for value in references {
            walk_strings(&value, &mut |raw| {
                if raw.contains("$inputs.") || raw.contains("$steps.") {
                    errors.push(format!(
                    "flow `{name}` hygiene violation: `{raw}` may reference only `$params.*` and `$pages.*`"
                ));
                }
                for param in param_reference_names(raw) {
                    if !flow.params.contains_key(param) {
                        errors.push(format!(
                            "flow `{name}` references unknown param `{param}` in `{raw}`"
                        ));
                    }
                }
            });
        }
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
        for called in flow
            .steps
            .iter()
            .flat_map(|s| std::iter::once(s).chain(s.steps.iter().flatten()))
            .filter_map(|step| step.call.as_deref())
        {
            visit(called, catalog, chain, depth + 1, errors);
        }
        chain.pop();
    }

    for name in catalog.keys() {
        visit(name, catalog, &mut Vec::new(), 0, errors);
    }
}
