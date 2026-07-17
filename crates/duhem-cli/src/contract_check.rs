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
        }
    }
    errs
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
}
