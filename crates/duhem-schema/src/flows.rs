//! Reusable flow validation and static expansion (spec #367).
//!
//! Expansion lives in the schema loader so the runtime continues to
//! execute one flat sequence of ordinary catalog actions. The authored
//! `flows:` catalog remains on the definition for round-tripping and
//! dashboard snapshot lookup; check and lifecycle steps are expanded.
use std::collections::{BTreeMap, BTreeSet};

use crate::assertion::Assertion;
use crate::includes::MAX_INCLUDE_DEPTH;
use crate::source::StepSourceOrigin;
use crate::step::{ExpandedFlowOrigin, Step, StepCondition};
use crate::verification::{Flow, FlowCatalog, InputDecl, InputType, VerificationDefinition};

#[path = "flows_validate.rs"]
mod validation;
pub(crate) use validation::validate_authored;

pub(crate) const MAX_FLOW_DEPTH: usize = MAX_INCLUDE_DEPTH;

/// Validate every flow-facing rule and expand all check invocations.
pub(crate) fn validate_and_expand(definition: &mut VerificationDefinition) -> Result<(), String> {
    let errors = validate_authored(definition);
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }

    let catalog = definition.flows.clone();
    for (criterion_index, criterion) in definition.criteria.iter_mut().enumerate() {
        for (check_index, check) in criterion.checks.iter_mut().enumerate() {
            let authored = std::mem::take(&mut check.steps);
            let mut counter = 0usize;
            let mut expanded = expand_sequence(
                authored,
                &catalog,
                "",
                None,
                None,
                &[],
                "steps",
                &mut counter,
            );
            expand_check_loops(
                &mut expanded.steps,
                &catalog,
                &mut counter,
                &mut expanded.projections,
            );
            check.steps = expanded.steps;
            rewrite_steps(&mut check.steps, &expanded.projections);
            for assertion in &mut check.assertions {
                rewrite_assertion(assertion, &expanded.projections);
            }
            if let Some(Some(session)) = &mut check.session {
                *session = rewrite_string(session, &expanded.projections);
            }
            definition.source_map.record_expanded_step_origins(
                criterion_index,
                check_index,
                expanded.origins,
            );
        }
    }

    // `for_each:` bodies (#443 Tier 1) get the same static flow
    // expansion as a check's own `call:` steps — a `call:` body is
    // namespaced and param-substituted once here, up front, so the
    // runtime never needs the flow catalog: it clones the resulting
    // flat template once per iteration and only substitutes the
    // `as:` binding at dispatch time. Best-effort: a malformed
    // `for_each` (wrong body-form count, depth > 1) is left
    // unexpanded here and reported by `crate::validate` before any
    // run reaches it.
    //
    // A direct (non-`for_each`) `call:` step in a lifecycle block
    // (#526) gets the same treatment as a check's own `steps:` list:
    // `expand_lifecycle_calls_in_list` splices the flow's flattened
    // body inline, in place of the call step, before `for_each`
    // bodies in the same list are expanded — so a `for_each:` step
    // later in the block already sees any earlier direct call's
    // projected outputs rewritten into its own `with:`/`for_each:`
    // source. One counter is threaded across every lifecycle list (and
    // every `for_each` body within it) so invocation ordinals stay
    // globally unique, matching check-step flow expansion.
    let mut lifecycle_counter = 0usize;
    let leaf_projections = expand_lifecycle_calls_in_list(
        &mut definition.setup,
        &catalog,
        "setup",
        &mut lifecycle_counter,
    );
    expand_for_each_in_list(&mut definition.setup, &catalog, &mut lifecycle_counter);
    expand_lifecycle_calls_in_list(
        &mut definition.teardown,
        &catalog,
        "setup",
        &mut lifecycle_counter,
    );
    expand_for_each_in_list(&mut definition.teardown, &catalog, &mut lifecycle_counter);
    for (fixture_name, lifecycle) in definition.fixtures.iter_mut() {
        let fixture_root = format!("fixture.{fixture_name}");
        expand_lifecycle_calls_in_list(
            &mut lifecycle.up,
            &catalog,
            &fixture_root,
            &mut lifecycle_counter,
        );
        expand_for_each_in_list(&mut lifecycle.up, &catalog, &mut lifecycle_counter);
        expand_lifecycle_calls_in_list(
            &mut lifecycle.down,
            &catalog,
            &fixture_root,
            &mut lifecycle_counter,
        );
        expand_for_each_in_list(&mut lifecycle.down, &catalog, &mut lifecycle_counter);
    }
    for criterion in &mut definition.criteria {
        rewrite_seed(&mut criterion.session, &leaf_projections);
        let mut seed_projections = leaf_projections.clone();
        seed_projections.extend(expand_lifecycle_calls_in_list(
            &mut criterion.setup,
            &catalog,
            "setup",
            &mut lifecycle_counter,
        ));
        expand_for_each_in_list(&mut criterion.setup, &catalog, &mut lifecycle_counter);
        expand_lifecycle_calls_in_list(
            &mut criterion.teardown,
            &catalog,
            "setup",
            &mut lifecycle_counter,
        );
        expand_for_each_in_list(&mut criterion.teardown, &catalog, &mut lifecycle_counter);
        for check in &mut criterion.checks {
            rewrite_seed(&mut check.session, &seed_projections);
            if let Some(named) = &mut check.sessions {
                for expression in named.values_mut().flatten() {
                    *expression = crate::ExprStr::from_source(&rewrite_string(
                        &expression.raw,
                        &seed_projections,
                    ))
                    .expect("projection preserves expression syntax");
                }
            }
            expand_lifecycle_calls_in_list(
                &mut check.setup,
                &catalog,
                "setup",
                &mut lifecycle_counter,
            );
            expand_for_each_in_list(&mut check.setup, &catalog, &mut lifecycle_counter);
            expand_lifecycle_calls_in_list(
                &mut check.teardown,
                &catalog,
                "setup",
                &mut lifecycle_counter,
            );
            expand_for_each_in_list(&mut check.teardown, &catalog, &mut lifecycle_counter);
        }
    }
    Ok(())
}

/// Expand a direct (non-`for_each`) `call:` step in one lifecycle step
/// list — leaf/criterion/check `setup:`/`teardown:` or fixture
/// `up:`/`down:` — into its flow's flattened steps, in place (#526).
/// Reuses the same static expansion a check's `steps:` list gets
/// (`expand_sequence`). `external_root` is the accessor lifecycle
/// steps in *this* list actually publish their outputs to at runtime
/// (`setup` for leaf/criterion/check blocks, `fixture.<name>` for a
/// fixture's own `up:`/`down:`) — never `steps`, which is check-only
/// (§10.7) — so a call's declared `outputs:` project onto
/// `$<external_root>.<call id>.outputs.<name>` and a later step in the
/// same block can read them the way it already reads any other
/// lifecycle step's output. Non-`call` steps, including `for_each:`
/// steps (expanded separately by `expand_for_each_in_list`), pass
/// through unchanged.
fn expand_lifecycle_calls_in_list(
    steps: &mut Vec<Step>,
    catalog: &FlowCatalog,
    external_root: &str,
    counter: &mut usize,
) -> BTreeMap<String, String> {
    let authored = std::mem::take(steps);
    let expanded = expand_sequence(
        authored,
        catalog,
        "",
        None,
        None,
        &[],
        external_root,
        counter,
    );
    *steps = expanded.steps;
    rewrite_steps(steps, &expanded.projections);
    expanded.projections
}

/// Resolve a `for_each:` step's authored body (exactly one of `uses:`,
/// `call:`, `steps:`) into the `Vec<Step>` `expand_sequence` expects.
/// `None` when the body shape is invalid — the caller leaves
/// `for_each_body` empty and `validate_for_each` reports the real
/// error with a source location.
fn for_each_body_steps(step: &Step) -> Option<Vec<Step>> {
    let forms = (
        step.uses.as_deref().filter(|u| !u.trim().is_empty()),
        step.call.as_deref().filter(|c| !c.trim().is_empty()),
        step.steps.as_ref(),
    );
    match forms {
        (Some(uses), None, None) => Some(vec![Step {
            uses: Some(uses.to_string()),
            with: step.with.clone(),
            session: step.session.clone(),
            ..blank_step()
        }]),
        (None, Some(call), None) => Some(vec![Step {
            call: Some(call.to_string()),
            with: step.with.clone(),
            session: step.session.clone(),
            ..blank_step()
        }]),
        (None, None, Some(body)) => Some(body.clone()),
        _ => None,
    }
}

/// A `Step` with every field at its wire-absent default. Used to
/// build the synthetic single-step body for the `uses:`/`call:`
/// for_each body forms without hand-listing every field.
fn blank_step() -> Step {
    Step {
        needs: Vec::new(),
        id: None,
        session: None,
        description: None,
        condition: StepCondition::Success,
        uses: None,
        call: None,
        with: serde_yml::Value::Null,
        outputs: BTreeMap::new(),
        secret_outputs: Vec::new(),
        for_each: None,
        max: None,
        as_binding: None,
        steps: None,
        for_each_body: Vec::new(),
        flow: None,
        flow_secrets: Vec::new(),
    }
}

/// Expand every `for_each:` step's body in one lifecycle step list
/// (leaf `setup:`/`teardown:`, fixture `up:`/`down:`, or a
/// criterion-/check-level `setup:`/`teardown:`). Non-`for_each` steps
/// are untouched. `counter` is threaded across every call so
/// invocation ordinals stay unique across the whole definition, same
/// as check-step flow expansion.
fn expand_for_each_in_list(steps: &mut [Step], catalog: &FlowCatalog, counter: &mut usize) {
    for step in steps.iter_mut() {
        if step.for_each.is_none() {
            continue;
        }
        let Some(body_steps) = for_each_body_steps(step) else {
            continue;
        };
        let ordinal = *counter;
        *counter += 1;
        let construct = step.call.clone().unwrap_or_else(|| "for_each".to_string());
        let invocation = step
            .id
            .clone()
            .unwrap_or_else(|| format!("for_each#{ordinal}"));
        let expanded = expand_sequence(
            body_steps,
            catalog,
            &invocation,
            Some(&construct),
            Some(&invocation),
            &[],
            "steps",
            counter,
        );
        let mut body = expanded.steps;
        rewrite_steps(&mut body, &expanded.projections);
        step.for_each_body = body;
    }
}

#[derive(Default)]
struct Expansion {
    steps: Vec<Step>,
    origins: Vec<StepSourceOrigin>,
    projections: BTreeMap<String, String>,
}

/// `external_root` is the accessor prefix (`steps`, `setup`, or
/// `fixture.<name>`) that a call step's *caller* would use to read the
/// call's declared `outputs:` — i.e. what `result.projections`' keys
/// and values are rooted in for calls processed directly by *this*
/// invocation. A flow's own body is always internally `$steps.`-rooted
/// (hygiene, independent of who calls it), so the recursive call that
/// expands a flow's body always passes `"steps"` regardless of what
/// `external_root` this invocation received (#526).
#[allow(clippy::too_many_arguments)]
fn expand_sequence(
    steps: Vec<Step>,
    catalog: &FlowCatalog,
    namespace: &str,
    current_flow: Option<&str>,
    current_invocation: Option<&str>,
    inherited_secrets: &[serde_yml::Value],
    external_root: &str,
    counter: &mut usize,
) -> Expansion {
    let mut result = Expansion::default();
    for (inner_index, mut step) in steps.into_iter().enumerate() {
        rewrite_value(&mut step.with, &result.projections);
        if step.call.is_none() || step.for_each.is_some() {
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
                    // Patched in per-clone by the `for_each` runtime
                    // expansion (#443 Tier 1); this static template has
                    // no iteration yet.
                    iteration: None,
                });
            }
            step.flow_secrets = inherited_secrets.to_vec();
            result.steps.push(step);
            result.origins.push(match current_flow {
                Some(name) => StepSourceOrigin::FlowDefinition {
                    name: name.to_string(),
                    step_index: inner_index,
                },
                None => StepSourceOrigin::AuthoredCheck {
                    step_index: inner_index,
                },
            });
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
            substitute_step_params(inner, &bindings);
            inherit_session(inner, step.session.as_deref());
        }
        let mut expanded = expand_sequence(
            body,
            catalog,
            &invocation,
            Some(flow_name),
            Some(&invocation),
            &secrets,
            "steps",
            counter,
        );
        for inner in &mut expanded.steps {
            inner.condition = step.condition.clone();
        }

        let mut direct_ids = BTreeMap::new();
        for inner in flow
            .steps
            .iter()
            .flat_map(|s| std::iter::once(s).chain(s.steps.iter().flatten()))
        {
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
            // `flow.outputs` is always authored `$steps.<inner
            // id>.outputs.<name>` (flow hygiene), and `direct_ids`
            // keeps that rooting through namespacing — so `projected`
            // is `$steps.<...>` here regardless of `external_root`.
            // Re-root it onto the accessor this call's own caller
            // actually reads from (`$steps.` unchanged for a check;
            // `$setup.`/`$fixture.<name>.` for a lifecycle block,
            // #526) before it becomes the caller-facing projection.
            let projected =
                rewrite_string(&rewrite_string(raw, &expanded.projections), &direct_ids);
            let projected = match projected.strip_prefix("$steps.") {
                Some(rest) if external_root != "steps" => format!("${external_root}.{rest}"),
                _ => projected,
            };
            if let Some(id) = &step.id {
                result
                    .projections
                    .insert(format!("${external_root}.{id}.outputs.{output}"), projected);
            }
        }
        result.steps.append(&mut expanded.steps);
        result.origins.append(&mut expanded.origins);
    }
    result
}

fn substitute_step_params(step: &mut Step, bindings: &BTreeMap<String, serde_yml::Value>) {
    substitute_params(&mut step.with, bindings);
    if let Some(expr) = &mut step.for_each {
        let mut value = serde_yml::Value::String(expr.raw.clone());
        substitute_params(&mut value, bindings);
        if let Some(raw) = value.as_str() {
            *expr = crate::ExprStr::from_source(raw).expect("validated flow source");
        }
    }
    if let Some(body) = &mut step.steps {
        for inner in body {
            substitute_step_params(inner, bindings);
        }
    }
}

/// Keep inline body ids addressable by the check's assertions. Flow calls
/// still namespace private ids and project their declared outputs normally.
fn expand_check_loops(
    steps: &mut [Step],
    catalog: &FlowCatalog,
    counter: &mut usize,
    projections: &mut BTreeMap<String, String>,
) {
    for step in steps {
        if step.for_each.is_none() {
            continue;
        }
        let Some(body) = for_each_body_steps(step) else {
            continue;
        };
        let ordinal = *counter;
        *counter += 1;
        let invocation = format!("for_each#{ordinal}");
        let namespace = step
            .flow
            .as_ref()
            .map(|f| f.invocation.as_str())
            .unwrap_or("");
        let mut expanded = expand_sequence(
            body,
            catalog,
            namespace,
            Some("for_each"),
            Some(&invocation),
            &step.flow_secrets,
            "steps",
            counter,
        );
        rewrite_steps(&mut expanded.steps, &expanded.projections);
        projections.extend(expanded.projections);
        step.for_each_body = expanded.steps;
    }
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
            if let Some(reference) = raw.strip_prefix("$params.") {
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
            } else if raw.trim_start().starts_with("$pages.")
                && let Some(rewritten) = rewrite_page_param_args(raw, bindings)
            {
                *raw = rewritten;
            }
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

/// Substitute flow params embedded as page-call arguments before the
/// expanded step reaches the ordinary expression parser (spec #495).
fn rewrite_page_param_args(
    raw: &str,
    bindings: &BTreeMap<String, serde_yml::Value>,
) -> Option<String> {
    let marker = "$params.";
    let mut rest = raw;
    let mut out = String::with_capacity(raw.len());
    let mut changed = false;
    while let Some(offset) = rest.find(marker) {
        out.push_str(&rest[..offset]);
        let reference = &rest[offset + marker.len()..];
        let name = reference_head(reference)?;
        let replacement = bindings.get(name).and_then(render_expr_value)?;
        out.push_str(&replacement);
        rest = &reference[name.len()..];
        changed = true;
    }
    out.push_str(rest);
    changed.then_some(out)
}

fn render_expr_value(value: &serde_yml::Value) -> Option<String> {
    match value {
        serde_yml::Value::String(raw)
            if raw.trim_start().starts_with('$') && crate::expr::parse(raw).is_ok() =>
        {
            Some(raw.clone())
        }
        serde_yml::Value::String(raw) if !raw.contains('\'') => Some(format!("'{raw}'")),
        serde_yml::Value::String(raw) if !raw.contains('"') => Some(format!("\"{raw}\"")),
        serde_yml::Value::String(raw) => {
            let mut args = Vec::new();
            for (index, part) in raw.split('\'').enumerate() {
                if index > 0 {
                    args.push("\"'\"".to_string());
                }
                args.push(format!("'{part}'"));
            }
            Some(format!("$runtime.concat({})", args.join(", ")))
        }
        serde_yml::Value::Bool(value) => Some(value.to_string()),
        serde_yml::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn rewrite_steps(steps: &mut [Step], projections: &BTreeMap<String, String>) {
    for step in steps {
        rewrite_value(&mut step.with, projections);
        if let Some(source) = &mut step.for_each {
            *source = crate::ExprStr::from_source(&rewrite_string(&source.raw, projections))
                .expect("rewriting a valid step reference preserves expression syntax");
        }
        rewrite_steps(&mut step.for_each_body, projections);
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

fn param_reference_names(raw: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = raw;
    while let Some(offset) = rest.find("$params.") {
        rest = &rest[offset + "$params.".len()..];
        let Some(name) = reference_head(rest) else {
            continue;
        };
        names.push(name);
        rest = &rest[name.len()..];
    }
    names
}

fn step_output_reference(raw: &str) -> Option<(&str, &str)> {
    let reference = raw.strip_prefix("$steps.")?;
    let (step, output) = reference.split_once(".outputs.")?;
    (!step.is_empty() && !output.is_empty()).then_some((step, output))
}

/// Fill browser actions and nested calls before expanding nested invocations.
fn inherit_session(step: &mut Step, session: Option<&str>) {
    if step.session.is_none()
        && (step.call.is_some()
            || step
                .uses
                .as_deref()
                .is_some_and(|uses| uses.starts_with("ui/")))
    {
        step.session = session.map(str::to_owned);
    }
    if let Some(body) = &mut step.steps {
        for inner in body {
            inherit_session(inner, session);
        }
    }
}

fn rewrite_seed(seed: &mut Option<Option<String>>, projections: &BTreeMap<String, String>) {
    if let Some(Some(raw)) = seed {
        *raw = rewrite_string(raw, projections);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored(yaml: &str) -> VerificationDefinition {
        VerificationDefinition::from_yaml_str(yaml).expect("parse")
    }

    /// #443/#526: criterion-/check-level `setup:`/`teardown:` dispatch
    /// was never validated before #443 — a `call:` there reached
    /// `Step::uses_name()` at runtime (which `.expect()`s `uses` is
    /// `Some`) and panicked instead of failing `duhem validate`. #526
    /// then made a *direct* `call:` (no `for_each:`) legitimate there
    /// too. Pin all three: a bare `call:` validates clean and expands
    /// (previously rejected with "only valid in a check"), the
    /// identical `call:` wrapped in `for_each:` still validates clean,
    /// and both get the flow's own param-type checking.
    #[test]
    fn criterion_and_check_level_call_dispatch_is_validated_not_left_to_panic() {
        let bare_call = authored(
            r#"
verification: x
flows:
  greet:
    steps:
      - uses: cli/invoke
criteria:
  - id: AC-1
    description: x
    setup:
      - call: greet
    checks:
      - id: AC-1.1
        steps: []
        assertions: ["true"]
"#,
        );
        assert!(
            validate_authored(&bare_call).is_empty(),
            "a direct `call:` in criterion setup: is legitimate as of #526: {:?}",
            validate_authored(&bare_call)
        );

        let for_each_call = authored(
            r#"
verification: x
inputs:
  rows: { type: array, default: [] }
flows:
  greet:
    params:
      name: { type: string }
    steps:
      - uses: cli/invoke
        with: { command: [echo, $params.name] }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        setup:
          - for_each: $inputs.rows
            max: 5
            as: row
            call: greet
            with: { name: $row }
        steps: []
        assertions: ["true"]
"#,
        );
        assert!(
            validate_authored(&for_each_call).is_empty(),
            "{:?}",
            validate_authored(&for_each_call)
        );

        // A mistyped flow param is still caught (proves real
        // flow-param validation runs here, not just a shape check).
        let bad_param = authored(
            r#"
verification: x
inputs:
  rows: { type: array, default: [] }
flows:
  greet:
    params:
      name: { type: string }
    steps:
      - uses: cli/invoke
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        setup:
          - for_each: $inputs.rows
            max: 5
            as: row
            call: greet
            with: { nam: $row }
        steps: []
        assertions: ["true"]
"#,
        );
        let errors = validate_authored(&bad_param).join("\n");
        assert!(errors.contains("missing parameter `name`"), "{errors}");
    }

    /// #526 Test item 1: a direct `call:` validates at each of the
    /// five lifecycle sites (leaf, criterion, check, fixture up,
    /// fixture down — `setup:`/`teardown:` counted together per
    /// site), and an unknown flow name or a missing/mistyped param is
    /// still rejected with a location, same as a check's own `call:`.
    #[test]
    fn direct_call_validates_at_every_lifecycle_site_and_flow_params_are_checked() {
        let ok = authored(
            r#"
verification: x
flows:
  greet:
    params:
      name: { type: string }
    steps:
      - uses: cli/invoke
        with: { command: [echo, $params.name] }
setup:
  - id: a
    call: greet
    with: { name: leaf-setup }
teardown:
  - id: b
    call: greet
    with: { name: leaf-teardown }
fixtures:
  server:
    up:
      - id: c
        call: greet
        with: { name: fixture-up }
    down:
      - id: d
        call: greet
        with: { name: fixture-down }
criteria:
  - id: AC-1
    description: x
    setup:
      - id: e
        call: greet
        with: { name: criterion-setup }
    teardown:
      - id: f
        call: greet
        with: { name: criterion-teardown }
    checks:
      - id: AC-1.1
        setup:
          - id: g
            call: greet
            with: { name: check-setup }
        teardown:
          - id: h
            call: greet
            with: { name: check-teardown }
        steps: []
        assertions: ["true"]
"#,
        );
        assert!(
            validate_authored(&ok).is_empty(),
            "{:?}",
            validate_authored(&ok)
        );

        let unknown_flow = authored(
            r#"
verification: x
setup:
  - call: missing
criteria: []
"#,
        );
        let errors = validate_authored(&unknown_flow).join("\n");
        assert!(errors.contains("unknown flow `missing`"), "{errors}");

        let missing_param = authored(
            r#"
verification: x
flows:
  greet:
    params:
      name: { type: string }
    steps:
      - uses: cli/invoke
fixtures:
  server:
    up:
      - call: greet
        with: {}
    down:
      - uses: cli/invoke
criteria: []
"#,
        );
        let errors = validate_authored(&missing_param).join("\n");
        assert!(errors.contains("missing parameter `name`"), "{errors}");
    }

    /// #526: a direct call's outputs project onto the lifecycle
    /// block's own accessor (`$setup.<call id>.outputs.<name>`, not
    /// `$steps.` — that root is check-only), so a later step in the
    /// *same* block reads them like it would any other lifecycle
    /// step's output.
    #[test]
    fn direct_call_output_is_reachable_by_a_later_step_in_the_same_block() {
        let mut definition = authored(
            r#"
verification: x
flows:
  make_token:
    steps:
      - id: produce
        uses: cli/invoke
        with: { command: [echo, made] }
    outputs:
      value: $steps.produce.outputs.stdout
setup:
  - id: make
    call: make_token
  - uses: cli/invoke
    with: { command: [echo, $setup.make.outputs.value] }
criteria: []
"#,
        );
        validate_and_expand(&mut definition).expect("expand");
        assert_eq!(definition.setup.len(), 2);
        assert_eq!(definition.setup[0].id.as_deref(), Some("make__produce"));
        assert_eq!(
            definition.setup[1].with["command"][1].as_str(),
            Some("$setup.make__produce.outputs.stdout"),
            "the later step's `$setup.make.outputs.value` reference must be rewritten onto \
             the real, namespaced setup-step id — not left pointing at `$steps.*`, which \
             lifecycle blocks never resolve"
        );
    }

    /// #526: the same projection, but for a fixture `up:` call whose
    /// declared output a later `up:` step reads via `$fixture.<name>.*`.
    #[test]
    fn direct_call_output_in_fixture_up_projects_onto_the_fixture_accessor() {
        let mut definition = authored(
            r#"
verification: x
flows:
  make_token:
    steps:
      - id: produce
        uses: cli/invoke
        with: { command: [echo, made] }
    outputs:
      value: $steps.produce.outputs.stdout
fixtures:
  server:
    up:
      - id: make
        call: make_token
      - uses: cli/invoke
        with: { command: [echo, $fixture.server.make.outputs.value] }
    down:
      - uses: cli/invoke
criteria: []
"#,
        );
        validate_and_expand(&mut definition).expect("expand");
        let up = &definition.fixtures["server"].up;
        assert_eq!(up.len(), 2);
        assert_eq!(up[0].id.as_deref(), Some("make__produce"));
        assert_eq!(
            up[1].with["command"][1].as_str(),
            Some("$fixture.server.make__produce.outputs.stdout")
        );
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
    fn expands_flow_param_inside_parameterized_page_reference() {
        let mut definition = authored(
            r#"
verification: flow page call
inputs:
  index: { type: integer, default: 2 }
pages:
  chat:
    history_item: { xpath: '(//article)[{}]' }
flows:
  select_history:
    params:
      index: { type: integer }
    steps:
      - uses: ui/assert-element
        with:
          locator: $pages.chat.history_item($params.index)
          expected: visible
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - call: select_history
            with: { index: $inputs.index }
"#,
        );
        validate_and_expand(&mut definition).expect("flow expands");
        assert_eq!(
            definition.criteria[0].checks[0].steps[0].with["locator"].as_str(),
            Some("$pages.chat.history_item($inputs.index)")
        );
        crate::validate(&definition).expect("expanded page call validates");
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

    #[test]
    fn for_each_uses_body_expands_to_a_single_templated_step_with_provenance() {
        let mut definition = authored(
            r#"
verification: for_each uses body
setup:
  - id: loop
    for_each: $inputs.rows
    max: 5
    as: row
    uses: cli/invoke
    with: { command: [echo, $row] }
criteria: []
"#,
        );
        validate_and_expand(&mut definition).expect("expand");
        let step = &definition.setup[0];
        assert_eq!(step.for_each_body.len(), 1);
        let body = &step.for_each_body[0];
        assert_eq!(body.uses.as_deref(), Some("cli/invoke"));
        assert_eq!(
            body.flow.as_ref().map(|f| (f.name.as_str(), f.iteration)),
            Some(("for_each", None)),
            "iteration is patched in per-clone at runtime, not at schema-expansion time"
        );
    }

    #[test]
    fn for_each_call_body_expands_the_flow_exactly_like_a_check_call_step() {
        let mut definition = authored(
            r#"
verification: for_each call body
flows:
  delete_row:
    params:
      row: { type: string }
    steps:
      - id: click
        uses: cli/invoke
        with: { command: [rm, $params.row] }
setup:
  - id: loop
    for_each: $inputs.rows
    max: 5
    as: row
    call: delete_row
    with: { row: $row }
criteria: []
"#,
        );
        validate_and_expand(&mut definition).expect("expand");
        let step = &definition.setup[0];
        assert_eq!(step.for_each_body.len(), 1);
        let body = &step.for_each_body[0];
        assert!(
            body.id.as_deref().is_some_and(|id| id.starts_with("loop__")
                && id.contains("delete_row")
                && id.ends_with("__click")),
            "expected a namespaced id under the `loop` invocation, got {:?}",
            body.id
        );
        assert_eq!(body.with["command"][1].as_str(), Some("$row"));
        assert_eq!(
            body.flow.as_ref().map(|f| f.name.as_str()),
            Some("delete_row")
        );
    }

    #[test]
    fn for_each_steps_body_expands_every_inner_step() {
        let mut definition = authored(
            r#"
verification: for_each steps body
setup:
  - for_each: $inputs.rows
    max: 5
    as: row
    steps:
      - uses: cli/invoke
        with: { command: [echo, first, $row] }
      - uses: cli/invoke
        with: { command: [echo, second] }
criteria: []
"#,
        );
        validate_and_expand(&mut definition).expect("expand");
        let step = &definition.setup[0];
        assert_eq!(step.for_each_body.len(), 2);
        assert_eq!(
            step.for_each_body[0].with["command"][2].as_str(),
            Some("$row")
        );
    }

    /// #526: a direct `call:` in a lifecycle block expands exactly
    /// like a check's own `call:` step — namespaced inner ids, `flow`
    /// provenance on every inner step — except the expanded steps
    /// carry no `iteration` (that field only ever gets patched in for
    /// a `for_each:` body, `for_each.rs::append_setup_started`; a
    /// direct call's static template is never cloned per element).
    #[test]
    fn direct_call_expands_with_flow_provenance_and_no_iteration() {
        let mut definition = authored(
            r#"
verification: x
flows:
  delete_skill:
    params:
      slug: { type: string }
    steps:
      - id: open
        uses: cli/invoke
        with: { command: [echo, open, $params.slug] }
      - id: delete
        uses: cli/invoke
        with: { command: [echo, delete, $params.slug] }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps: []
        teardown:
          - id: cleanup
            call: delete_skill
            with: { slug: my-skill }
        assertions: ["true"]
"#,
        );
        validate_and_expand(&mut definition).expect("expand");
        let teardown = &definition.criteria[0].checks[0].teardown;
        assert_eq!(teardown.len(), 2);
        assert_eq!(teardown[0].id.as_deref(), Some("cleanup__open"));
        assert_eq!(teardown[1].id.as_deref(), Some("cleanup__delete"));
        for step in teardown {
            let flow = step.flow.as_ref().expect("flow provenance set");
            assert_eq!(flow.name, "delete_skill");
            assert_eq!(flow.invocation, "cleanup");
            assert_eq!(flow.iteration, None, "a direct call is never an iteration");
        }
        assert_eq!(teardown[0].flow.as_ref().unwrap().inner_index, 0);
        assert_eq!(teardown[1].flow.as_ref().unwrap().inner_index, 1);
    }
}
