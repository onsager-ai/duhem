//! Validate-time field checking against the action contract (spec #247).
//!
//! `duhem_schema::validate` checks reference *wiring* (`$steps.<id>` ids
//! and declared outputs); this adds *field accuracy* — each step's `with:`
//! keys, the action-field on the right of `outputs:`, and closed-enum
//! values are cross-checked against the action's `contract()`, so a typo
//! fails at validate time with a prescriptive "valid: …" hint instead of
//! only at run time.
//!
//! Layered in the CLI, not in `duhem-schema`, because the action contracts
//! live in `duhem-actions` (which `duhem-schema` does not depend on).
//!
//! **Under-enforces by design** so a valid VD is never false-rejected: an
//! unknown/custom `uses` (no contract) is skipped, and the `outputs:` check
//! runs only when the contract lists outputs. The run-time
//! `deny_unknown_fields` remains the backstop for anything not caught here.

use duhem_actions::contract_for;
use duhem_schema::{Step, VerificationDefinition};

/// Field-accuracy errors across every step (setup + criteria/checks).
/// Empty = clean.
pub(crate) fn field_errors(def: &VerificationDefinition) -> Vec<String> {
    let mut errs = Vec::new();
    for (i, s) in def.setup.iter().enumerate() {
        check_step(s, &format!("setup step {i}"), &mut errs);
    }
    for c in &def.criteria {
        for ch in &c.checks {
            for (i, s) in ch.steps.iter().enumerate() {
                let site = format!("criterion `{}` / check `{}` / step {i}", c.id, ch.id);
                check_step(s, &site, &mut errs);
            }
            check_judgment(c, ch, &mut errs);
        }
    }
    errs
}

/// A check with no `assertions:` must carry its verdict through at
/// least one implicitly-judging step (spec #253): an action whose
/// contract emits a boolean `satisfied` output, with `satisfied` NOT
/// bound in the step's `outputs:` (binding it takes manual control).
/// Under-enforces like the field checks: a step with an unknown
/// `uses` (no contract) counts as potentially judging, so a custom
/// action is never false-rejected here — the runtime surfaces it as
/// `Inconclusive` if it turns out not to exist.
fn check_judgment(c: &duhem_schema::Criterion, ch: &duhem_schema::Check, errs: &mut Vec<String>) {
    if !ch.assertions.is_empty() || ch.steps.is_empty() {
        return; // explicit assertions, or schema-level NothingToJudge.
    }
    let any_judging = ch.steps.iter().any(|s| match contract_for(&s.uses) {
        None => true, // unknown/custom action — assume it may judge.
        // Binding an output named `satisfied` is the manual-control
        // opt-out (mirrors the runtime in `implicit_judgment_outcomes`).
        Some(contract) => contract.judges() && !s.outputs.contains_key("satisfied"),
    });
    if !any_judging {
        errs.push(format!(
            "criterion `{}` / check `{}`: no `assertions:` and no judging step — add an assertion, or a step whose action emits `satisfied` (e.g. ui/assert-*, api/poll) without binding `satisfied` in `outputs:`",
            c.id, ch.id
        ));
    }
}

fn check_step(s: &Step, site: &str, errs: &mut Vec<String>) {
    let Some(contract) = contract_for(&s.uses) else {
        return; // unknown/custom action — leave to run-time deny_unknown_fields.
    };
    let uses = &s.uses;

    // 1. `with:` keys + 3. closed-enum values.
    if let serde_yml::Value::Mapping(m) = &s.with {
        for (key, val) in m {
            let Some(k) = key.as_str() else { continue };
            match contract.with.iter().find(|f| f.name == k) {
                None => {
                    let valid: Vec<&str> = contract.with.iter().map(|f| f.name).collect();
                    errs.push(format!(
                        "{site}: `{uses}` has no `with:` field `{k}` (valid: {})",
                        valid.join(", ")
                    ));
                }
                Some(f) if !f.enum_values.is_empty() => {
                    if let Some(v) = val.as_str().filter(|v| !f.enum_values.contains(v)) {
                        errs.push(format!(
                            "{site}: `{uses}` field `{k}` = `{v}` is not valid (one of: {})",
                            f.enum_values.join(", ")
                        ));
                    }
                }
                Some(_) => {}
            }
        }
    }

    // 2. `outputs:` action-fields — only when the contract lists outputs.
    if !contract.outputs.is_empty() {
        for (local, field) in &s.outputs {
            if !contract.outputs.contains(&field.as_str()) {
                errs.push(format!(
                    "{site}: `{uses}` produces no output `{field}` (bound as `{local}`; valid: {})",
                    contract.outputs.join(", ")
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vd(steps_yaml: &str) -> VerificationDefinition {
        let src = format!(
            "verification: t\ncriteria:\n  - id: AC-1\n    description: d\n    checks:\n      - id: AC-1.1\n        description: d\n        steps:\n{steps_yaml}\n        assertions: [\"1 == 1\"]\n"
        );
        VerificationDefinition::from_yaml_str(&src).expect("parse")
    }

    #[test]
    fn valid_step_has_no_errors() {
        let d = vd(
            "          - { id: h, uses: api/call, with: { method: GET, url: u }, outputs: { s: status } }",
        );
        assert!(field_errors(&d).is_empty(), "{:?}", field_errors(&d));
    }

    #[test]
    fn unknown_with_key_flagged() {
        let d = vd("          - { uses: api/call, with: { method: GET, url: u, bogus: 1 } }");
        assert!(
            field_errors(&d)
                .iter()
                .any(|m| m.contains("no `with:` field `bogus`")),
            "{:?}",
            field_errors(&d)
        );
    }

    #[test]
    fn unknown_output_field_flagged() {
        let d = vd(
            "          - { uses: api/call, with: { method: GET, url: u }, outputs: { x: nope } }",
        );
        assert!(
            field_errors(&d)
                .iter()
                .any(|m| m.contains("produces no output `nope`")),
            "{:?}",
            field_errors(&d)
        );
    }

    #[test]
    fn bad_enum_value_flagged() {
        let d = vd(
            "          - { uses: ui/assert-element, with: { locator: { css: h1 }, expected: shown } }",
        );
        assert!(
            field_errors(&d)
                .iter()
                .any(|m| m.contains("`expected` = `shown`")),
            "{:?}",
            field_errors(&d)
        );
    }

    #[test]
    fn unknown_action_is_skipped() {
        // No contract → no field errors (run-time is the backstop).
        let d = vd("          - { uses: custom/thing, with: { anything: 1 } }");
        assert!(field_errors(&d).is_empty());
    }

    // ---- implicit judgment (spec #253) ----

    /// Like `vd()` but with NO `assertions:` on the check.
    fn vd_no_assertions(steps_yaml: &str) -> VerificationDefinition {
        let src = format!(
            "verification: t\ncriteria:\n  - id: AC-1\n    description: d\n    checks:\n      - id: AC-1.1\n        description: d\n        steps:\n{steps_yaml}\n"
        );
        VerificationDefinition::from_yaml_str(&src).expect("parse")
    }

    #[test]
    fn no_assertions_with_judging_step_is_accepted() {
        let d = vd_no_assertions(
            "          - { uses: ui/assert-element, with: { locator: { css: h1 }, expected: visible } }",
        );
        assert!(field_errors(&d).is_empty(), "{:?}", field_errors(&d));
    }

    #[test]
    fn no_assertions_without_judging_step_is_rejected() {
        let d = vd_no_assertions("          - { uses: api/call, with: { method: GET, url: u } }");
        assert!(
            field_errors(&d)
                .iter()
                .any(|m| m.contains("no judging step")),
            "{:?}",
            field_errors(&d)
        );
    }

    #[test]
    fn no_assertions_with_unknown_action_is_accepted() {
        // Under-enforce: a custom action may judge; the runtime
        // surfaces a truly-unknown `uses` as Inconclusive.
        let d = vd_no_assertions("          - { uses: custom/thing, with: { x: 1 } }");
        assert!(field_errors(&d).is_empty(), "{:?}", field_errors(&d));
    }

    #[test]
    fn no_assertions_opt_out_keys_on_output_name_not_extraction() {
        // Binding some *other* name to the `satisfied` extraction does
        // NOT opt out — the step still judges implicitly, so a
        // no-assertions check with it is accepted.
        let d = vd_no_assertions(
            "          - { id: s, uses: ui/assert-element, with: { locator: { css: h1 }, expected: visible }, outputs: { count: satisfied } }",
        );
        assert!(field_errors(&d).is_empty(), "{:?}", field_errors(&d));
    }

    #[test]
    fn no_assertions_with_satisfied_bound_is_rejected() {
        // Binding `satisfied` takes manual control — with no explicit
        // assertions left, nothing judges the check.
        let d = vd_no_assertions(
            "          - { id: s, uses: ui/assert-element, with: { locator: { css: h1 }, expected: visible }, outputs: { satisfied: satisfied } }",
        );
        assert!(
            field_errors(&d)
                .iter()
                .any(|m| m.contains("no judging step")),
            "{:?}",
            field_errors(&d)
        );
    }
}
