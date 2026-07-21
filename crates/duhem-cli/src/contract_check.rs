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
use duhem_schema::{
    Assertion, BinOp, Check, Expr, Literal, PathRoot, Step, VerificationDefinition,
};

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

/// The action-contract output resolver for `duhem_schema`'s
/// implicit-output inference (spec #267): the output names a `uses:`
/// declares, or empty for an unknown/custom action (which then resolves
/// against the step's authored `outputs:` only). Passed to
/// `duhem_schema::validate_with_contract_outputs` so `duhem validate` /
/// `run` accept `$steps.<id>.outputs.<field>` with no `outputs:` block.
pub(crate) fn contract_outputs(uses: &str) -> Vec<String> {
    contract_for(uses)
        .map(|c| c.outputs.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

/// Non-fatal authoring lints (spec #267) — the ceremony the terser
/// forms made unnecessary. Warnings, never errors: every VD valid today
/// stays valid, and existing consumer suites (crawlab-pro, chreode)
/// don't break; the author just sees where the plumbing can go. Empty =
/// clean.
///
/// Two patterns, both keyed off the action contract, so a custom `uses`
/// with no contract is never flagged (its bindings are load-bearing —
/// inference can't cover an output the catalog doesn't know):
///   (a) `outputs: { satisfied: satisfied }` on a judging step *paired
///       with* a `$steps.<id>.outputs.satisfied == true` assertion — the
///       hand-rolled, pre-#253 form of implicit judgment. The binding
///       even opts the step *out* of implicit judgment, so the author
///       re-adds by hand exactly what dropping both would give for free.
///   (b) an identity `outputs: { foo: foo }` binding whose `foo` is a
///       declared contract output — #267 inference resolves
///       `$steps.<id>.outputs.foo` with no binding.
/// A genuine rename/alias (`outputs: { n: row_count }`) is never
/// flagged; neither is a bare `satisfied` binding without the paired
/// `== true` (that is the legitimate manual-control seam).
pub(crate) fn lint_warnings(def: &VerificationDefinition) -> Vec<String> {
    let mut warns = Vec::new();
    for (i, s) in def.setup.iter().enumerate() {
        // Setup steps don't judge — only the identity-output lint applies.
        lint_step(s, None, &format!("setup step {i}"), &mut warns);
    }
    for c in &def.criteria {
        for ch in &c.checks {
            for (i, s) in ch.steps.iter().enumerate() {
                let site = format!("criterion `{}` / check `{}` / step {i}", c.id, ch.id);
                lint_step(s, Some(ch), &site, &mut warns);
            }
        }
    }
    warns
}

/// Emit the redundancy warnings for one step. `check` is the enclosing
/// check (for the `satisfied == true` pairing); `None` for a setup step.
fn lint_step(s: &Step, check: Option<&Check>, site: &str, warns: &mut Vec<String>) {
    let Some(contract) = contract_for(&s.uses) else {
        return; // custom action — no contract, bindings are load-bearing.
    };
    for (local, field) in &s.outputs {
        if local != field {
            continue; // a genuine rename/alias — legitimate, never flagged.
        }
        if field == "satisfied" {
            // (a) only when paired with an explicit `== true` assertion;
            // a bare `satisfied` binding is the manual-control opt-out.
            let paired = matches!((check, &s.id), (Some(ch), Some(id))
                if ch.assertions.iter().any(|a| asserts_satisfied_true(a, id)));
            if paired {
                let id = s.id.as_deref().unwrap_or("<step>");
                warns.push(format!(
                    "{site}: `outputs: {{ satisfied: satisfied }}` + a `$steps.{id}.outputs.satisfied == true` assertion re-implements implicit judgment (#253) by hand — drop both; a judging step with no `satisfied` binding is asserted `== true` automatically"
                ));
            }
            continue;
        }
        // (b) identity binding of a declared contract output — inference
        // (#267) resolves the reference without it.
        if contract.outputs.contains(&field.as_str()) {
            warns.push(format!(
                "{site}: identity binding `outputs: {{ {field}: {field} }}` is redundant — `$steps.<id>.outputs.{field}` resolves without it (#267); drop the binding"
            ));
        }
    }
}

/// Is `a` the assertion `$steps.<step_id>.outputs.satisfied == true`
/// (either operand order)? Matched on the parsed AST, not raw text, so
/// whitespace and `true == $…` don't slip past.
fn asserts_satisfied_true(a: &Assertion, step_id: &str) -> bool {
    let Assertion::Expr(e) = a else { return false };
    let Expr::BinOp {
        op: BinOp::Eq,
        lhs,
        rhs,
    } = &e.parsed
    else {
        return false;
    };
    let is_sat_path = |e: &Expr| {
        matches!(e, Expr::Path(p)
            if p.root == PathRoot::Steps
            && p.segments.len() == 3
            && p.segments[0] == step_id
            && p.segments[1] == "outputs"
            && p.segments[2] == "satisfied")
    };
    let is_true = |e: &Expr| matches!(e, Expr::Lit(Literal::Bool(true)));
    (is_sat_path(lhs) && is_true(rhs)) || (is_true(lhs) && is_sat_path(rhs))
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

    // ---- redundancy lints (spec #267 part 3) ----

    #[test]
    fn identity_output_binding_warns() {
        // `outputs: { status: status }` binds a declared contract output
        // to its own name — #267 inference covers it, so it's redundant.
        let d = vd(
            "          - { id: h, uses: api/call, with: { method: GET, url: u }, outputs: { status: status } }",
        );
        assert!(
            lint_warnings(&d)
                .iter()
                .any(|m| m.contains("identity binding") && m.contains("status")),
            "{:?}",
            lint_warnings(&d)
        );
    }

    #[test]
    fn rename_binding_not_warned() {
        // A genuine alias (`n` != `row_count`) is the escape hatch, never flagged.
        let d = vd(
            "          - { id: q, uses: db/query, with: { connection: c, sql: s }, outputs: { n: row_count } }",
        );
        assert!(lint_warnings(&d).is_empty(), "{:?}", lint_warnings(&d));
    }

    #[test]
    fn identity_binding_on_unknown_action_not_warned() {
        // No contract → the binding is load-bearing; inference can't cover it.
        let d =
            vd("          - { id: c, uses: custom/thing, with: { x: 1 }, outputs: { foo: foo } }");
        assert!(lint_warnings(&d).is_empty(), "{:?}", lint_warnings(&d));
    }

    #[test]
    fn satisfied_pair_warns() {
        // `outputs: { satisfied: satisfied }` + `… satisfied == true` is
        // implicit judgment (#253) re-implemented by hand.
        let src = "verification: t\ncriteria:\n  - id: AC-1\n    description: d\n    checks:\n      - id: AC-1.1\n        description: d\n        steps:\n          - { id: s, uses: ui/assert-element, with: { locator: { css: h1 }, expected: visible }, outputs: { satisfied: satisfied } }\n        assertions:\n          - $steps.s.outputs.satisfied == true\n";
        let d = VerificationDefinition::from_yaml_str(src).expect("parse");
        assert!(
            lint_warnings(&d)
                .iter()
                .any(|m| m.contains("implicit judgment")),
            "{:?}",
            lint_warnings(&d)
        );
    }

    #[test]
    fn bare_satisfied_binding_not_warned() {
        // Binding `satisfied` without the paired `== true` is the legit
        // manual-control seam (here asserting `== false`) — not flagged.
        let src = "verification: t\ncriteria:\n  - id: AC-1\n    description: d\n    checks:\n      - id: AC-1.1\n        description: d\n        steps:\n          - { id: s, uses: ui/assert-element, with: { locator: { css: h1 }, expected: visible }, outputs: { satisfied: satisfied } }\n        assertions:\n          - $steps.s.outputs.satisfied == false\n";
        let d = VerificationDefinition::from_yaml_str(src).expect("parse");
        assert!(lint_warnings(&d).is_empty(), "{:?}", lint_warnings(&d));
    }
}
