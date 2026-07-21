//! Structural validator for a parsed `VerificationDefinition`.
//!
//! Enforces well-formedness rules that `serde` alone can't express:
//! uniqueness of authored ids within their scope, and that every
//! `$steps.*` and `$inputs.*` reference inside an assertion resolves
//! to something declared in the same definition. Operator/type
//! checking is *not* done here — output value types aren't known
//! statically; the runtime spec owns evaluation.

use std::collections::{BTreeMap, HashMap, HashSet};

use thiserror::Error;

use crate::criterion::{Check, Criterion};
use crate::expr::{Expr, Path, PathRoot};
use crate::step::Step;
use crate::verification::{InputDecl, InputType, VerificationDefinition};

/// Where a `$...` reference was authored. Renders into a
/// [`ValidationError`] message so a `with:` ref names its step rather
/// than masquerading as an assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefSite {
    /// Inside a check's `assertions:` list.
    Assertion,
    /// Inside a step's `with:` payload. Carries the step's label —
    /// its `id` when declared, else `step <index>`.
    StepWith { step: String },
}

impl std::fmt::Display for RefSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefSite::Assertion => write!(f, "assertion"),
            RefSite::StepWith { step } => write!(f, "step `{step}` with:"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("verification has no criteria")]
    NoCriteria,

    #[error("duplicate criterion id `{id}`")]
    DuplicateCriterionId { id: String },

    #[error("criterion `{criterion}`: duplicate check id `{id}`")]
    DuplicateCheckId { criterion: String, id: String },

    #[error(
        "criterion `{criterion}` / check `{check}`: nothing to judge — no assertions and no steps (spec #253: a check may omit `assertions:` only when a judging step carries the verdict)"
    )]
    NothingToJudge { criterion: String, check: String },

    #[error("criterion `{criterion}` / check `{check}`: duplicate step id `{id}`")]
    DuplicateStepId {
        criterion: String,
        check: String,
        id: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: step `{step}` output `{name}` uses the reserved `capture/` prefix (runner-emitted failure evidence, spec #202)"
    )]
    ReservedOutputPrefix {
        criterion: String,
        check: String,
        step: String,
        name: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references undeclared step `{step}`"
    )]
    UnresolvedStepRef {
        criterion: String,
        check: String,
        step: String,
        raw: String,
        site: RefSite,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references undeclared output `{output}` on step `{step}`"
    )]
    UnresolvedStepOutput {
        criterion: String,
        check: String,
        step: String,
        output: String,
        raw: String,
        site: RefSite,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references undeclared input `{input}`"
    )]
    UnresolvedInputRef {
        criterion: String,
        check: String,
        input: String,
        raw: String,
        site: RefSite,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}`: malformed `$steps` reference (expected `$steps.<step_id>.outputs.<output>`)"
    )]
    MalformedStepRef {
        criterion: String,
        check: String,
        raw: String,
        site: RefSite,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}`: malformed `$inputs` reference (expected `$inputs.<name>`)"
    )]
    MalformedInputRef {
        criterion: String,
        check: String,
        raw: String,
        site: RefSite,
    },

    #[error("setup: duplicate step id `{id}`")]
    DuplicateSetupStepId { id: String },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references undeclared setup step `{step}`"
    )]
    UnresolvedSetupStepRef {
        criterion: String,
        check: String,
        step: String,
        raw: String,
        site: RefSite,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}` references undeclared output `{output}` on setup step `{step}`"
    )]
    UnresolvedSetupStepOutput {
        criterion: String,
        check: String,
        step: String,
        output: String,
        raw: String,
        site: RefSite,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: {site} `{raw}`: malformed `$setup` reference (expected `$setup.<step_id>.outputs.<output>`)"
    )]
    MalformedSetupRef {
        criterion: String,
        check: String,
        raw: String,
        site: RefSite,
    },

    #[error(
        "input `{input}`: default value type `{actual}` does not match declared type `{declared}`"
    )]
    InputDefaultTypeMismatch {
        input: String,
        declared: InputType,
        actual: String,
    },

    #[error(
        "inherited input `{name}` is also declared locally under `inputs:` — list it in one place, not both"
    )]
    InheritedInputAlsoDeclared { name: String },

    #[error("`inherits:` entry #{index} is empty — list input names, e.g. `inherits: [base_url]`")]
    EmptyInheritedName { index: usize },

    #[error("{0}")]
    BadProjectDecl(String),
}

/// Run every structural rule. Always reports as many errors as
/// possible — the goal is one round-trip from "save the file" to "see
/// the punch list".
pub fn validate(v: &VerificationDefinition) -> Result<(), Vec<ValidationError>> {
    // No contract knowledge: a `$steps.<id>.outputs.<name>` reference
    // resolves only against the step's authored `outputs:`. The
    // contract-aware CLI layer calls `validate_with_contract_outputs`
    // to *also* resolve against the action's declared outputs, so the
    // identity binding `outputs: { foo: foo }` becomes unnecessary
    // (spec #267). `duhem-schema` can't depend on `duhem-actions`, so
    // the catalog is injected as a closure.
    validate_with_contract_outputs(v, &|_| Vec::new())
}

/// Like [`validate`], but resolves `$steps.<id>.outputs.<name>` against
/// the step action's declared outputs (via `outputs_for`) in addition
/// to the step's authored `outputs:` map — the implicit-output
/// inference of spec #267. `outputs_for(uses)` returns the output names
/// the action contract declares, or empty for an unknown/custom action
/// (which is then resolved against authored outputs only, as before).
pub fn validate_with_contract_outputs(
    v: &VerificationDefinition,
    outputs_for: &dyn Fn(&str) -> Vec<String>,
) -> Result<(), Vec<ValidationError>> {
    let mut errs = Vec::new();

    if v.criteria.is_empty() {
        errs.push(ValidationError::NoCriteria);
    }

    // `project:` (#191): exactly one non-empty coordinate field.
    if let Some(project) = &v.project
        && let Err(msg) = project.check()
    {
        errs.push(ValidationError::BadProjectDecl(msg));
    }

    let setup_outputs = collect_setup_outputs(&v.setup, outputs_for, &mut errs);

    for (name, decl) in &v.inputs {
        if let Some(default) = &decl.default {
            let unwrapped = unwrap_tagged(default);
            match yml_shape(unwrapped) {
                Some(actual)
                    if actual != decl.kind && !matches_with_promotion(decl.kind, actual) =>
                {
                    errs.push(ValidationError::InputDefaultTypeMismatch {
                        input: name.clone(),
                        declared: decl.kind,
                        actual: yml_shape_name(unwrapped).to_string(),
                    });
                }
                // `null` doesn't map to any catalog member; `default:
                // null` is a way to express "no default" that we
                // don't support yet (optional inputs are a follow-up
                // spec). Reject it now rather than letting `null`
                // leak into the engine as a synthetic value.
                None => {
                    errs.push(ValidationError::InputDefaultTypeMismatch {
                        input: name.clone(),
                        declared: decl.kind,
                        actual: yml_shape_name(unwrapped).to_string(),
                    });
                }
                _ => {}
            }
        }
    }

    // Inherited input names (spec #135). They satisfy `$inputs.<name>`
    // references just like a locally-declared input, so they join the
    // resolvable-name set below. Two well-formedness rules first: a name
    // may not appear in both `inputs:` and `inherits:` (declare it once),
    // and an inherited name may not be empty.
    let mut inherited: HashSet<&str> = HashSet::new();
    for (index, name) in v.inherits.iter().enumerate() {
        if name.is_empty() {
            errs.push(ValidationError::EmptyInheritedName { index });
            continue;
        }
        if v.inputs.contains_key(name) {
            errs.push(ValidationError::InheritedInputAlsoDeclared { name: name.clone() });
        }
        inherited.insert(name.as_str());
    }

    let mut seen_criteria: HashSet<&str> = HashSet::new();
    for c in &v.criteria {
        if !seen_criteria.insert(c.id.as_str()) {
            errs.push(ValidationError::DuplicateCriterionId { id: c.id.clone() });
        }
        validate_criterion(
            c,
            &v.inputs,
            &inherited,
            &setup_outputs,
            outputs_for,
            &mut errs,
        );
    }

    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

/// A step's referenceable outputs (spec #267): the names it binds in
/// `outputs:` unioned with the outputs its action contract declares
/// (via `outputs_for`). The union is what lets `$steps.<id>.outputs.foo`
/// resolve without an `outputs: { foo: foo }` binding whenever `foo` is
/// a declared contract output; an explicit binding (a rename, or a
/// nested-extraction alias) still contributes its own name.
fn effective_outputs(s: &Step, outputs_for: &dyn Fn(&str) -> Vec<String>) -> HashSet<String> {
    let mut set: HashSet<String> = s.outputs.keys().cloned().collect();
    set.extend(outputs_for(&s.uses));
    set
}

/// Walk the run-level `setup:` block. Enforces id-uniqueness and
/// returns the map of `step_id → referenceable outputs` (authored ∪
/// contract, per [`effective_outputs`]) so per-check assertion
/// validation can resolve `$setup.<id>.outputs.<name>` references.
fn collect_setup_outputs<'a>(
    setup: &'a [Step],
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    errs: &mut Vec<ValidationError>,
) -> HashMap<&'a str, HashSet<String>> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut outputs: HashMap<&str, HashSet<String>> = HashMap::new();
    for s in setup {
        if let Some(id) = &s.id {
            if !seen.insert(id.as_str()) {
                errs.push(ValidationError::DuplicateSetupStepId { id: id.clone() });
            }
            outputs.insert(id.as_str(), effective_outputs(s, outputs_for));
        }
    }
    outputs
}

/// Classify a YAML default's structural shape against the input
/// catalog. Returns `None` only for `Null` — every other shape has a
/// catalog member. `Tagged` is unwrapped by [`unwrap_tagged`] before
/// reaching here.
///
/// Out-of-`i64`-range numerics classify as `Number`, not `Integer`,
/// because the downstream pipeline (`coerce_input` for `--inputs`,
/// runtime `Value::Int(i64)`) can't represent them as integers; a
/// `default: <huge>` under `type: integer` is a real mismatch and
/// authors should see the error at validate time.
fn yml_shape(v: &serde_yml::Value) -> Option<InputType> {
    use serde_yml::Value as Y;
    match v {
        Y::String(_) => Some(InputType::String),
        Y::Bool(_) => Some(InputType::Boolean),
        Y::Number(n) => {
            if n.is_i64() {
                Some(InputType::Integer)
            } else {
                Some(InputType::Number)
            }
        }
        Y::Sequence(_) => Some(InputType::Array),
        Y::Mapping(_) => Some(InputType::Object),
        Y::Null => None,
        // Unreachable after `unwrap_tagged`; defensive `None` keeps
        // the match total without panicking on a malformed tree.
        Y::Tagged(_) => None,
    }
}

/// Peel YAML `!tag scalar` wrappers (e.g. `!!str 3`) down to the
/// underlying value so the catalog classifier sees the real shape.
fn unwrap_tagged(v: &serde_yml::Value) -> &serde_yml::Value {
    let mut cur = v;
    while let serde_yml::Value::Tagged(t) = cur {
        cur = &t.value;
    }
    cur
}

/// Human-readable wire name for a default's actual shape. Used only
/// for the error message; the comparison itself goes through
/// `yml_shape`.
fn yml_shape_name(v: &serde_yml::Value) -> &'static str {
    use serde_yml::Value as Y;
    match v {
        Y::String(_) => "string",
        Y::Bool(_) => "boolean",
        Y::Number(n) => {
            if n.is_i64() {
                "integer"
            } else {
                "number"
            }
        }
        Y::Sequence(_) => "array",
        Y::Mapping(_) => "object",
        Y::Null => "null",
        Y::Tagged(_) => "tagged",
    }
}

/// An integer is also a valid `number` (no fractional part required).
/// The reverse — a fractional `number` under `type: integer` — is a
/// real mismatch and falls through to the error path.
fn matches_with_promotion(declared: InputType, actual: InputType) -> bool {
    matches!((declared, actual), (InputType::Number, InputType::Integer))
}

fn validate_criterion(
    c: &Criterion,
    inputs: &BTreeMap<String, InputDecl>,
    inherited: &HashSet<&str>,
    setup_outputs: &HashMap<&str, HashSet<String>>,
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    errs: &mut Vec<ValidationError>,
) {
    let mut seen_checks: HashSet<&str> = HashSet::new();
    for ch in &c.checks {
        if !seen_checks.insert(ch.id.as_str()) {
            errs.push(ValidationError::DuplicateCheckId {
                criterion: c.id.clone(),
                id: ch.id.clone(),
            });
        }
        validate_check(c, ch, inputs, inherited, setup_outputs, outputs_for, errs);
    }
}

/// Per-check view of resolvable references — bundled to keep
/// `check_path`'s arity within clippy's `too_many_arguments` lint.
struct PathScope<'a> {
    c: &'a Criterion,
    ch: &'a Check,
    step_outputs: &'a HashMap<&'a str, HashSet<String>>,
    setup_outputs: &'a HashMap<&'a str, HashSet<String>>,
    inputs: &'a BTreeMap<String, InputDecl>,
    /// Inherited input names (spec #135). An `$inputs.<name>` for a
    /// name in here resolves like a declared input — the manifest's
    /// chain binds it at run time.
    inherited: &'a HashSet<&'a str>,
}

fn validate_check(
    c: &Criterion,
    ch: &Check,
    inputs: &BTreeMap<String, InputDecl>,
    inherited: &HashSet<&str>,
    setup_outputs: &HashMap<&str, HashSet<String>>,
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    errs: &mut Vec<ValidationError>,
) {
    // A check with neither assertions nor steps can never produce a
    // verdict (the judge would see an empty aggregation). With steps
    // but no assertions the schema layer accepts — whether one of the
    // steps is a judging action (implicit judgment, spec #253) is a
    // catalog question the contract-aware CLI layer answers.
    if ch.assertions.is_empty() && ch.steps.is_empty() {
        errs.push(ValidationError::NothingToJudge {
            criterion: c.id.clone(),
            check: ch.id.clone(),
        });
    }

    let mut step_outputs: HashMap<&str, HashSet<String>> = HashMap::new();
    let mut seen_step_ids: HashSet<&str> = HashSet::new();

    for (idx, s) in ch.steps.iter().enumerate() {
        // The `capture/` output namespace is reserved for runner-emitted
        // failure evidence (spec #202) so authored outputs can never
        // masquerade as captures. Enforced here at the authoring
        // boundary — no action produces `capture/*`, so the runtime is
        // the only source.
        for name in s.outputs.keys() {
            if name.starts_with("capture/") {
                errs.push(ValidationError::ReservedOutputPrefix {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    step: step_label(s, idx),
                    name: name.clone(),
                });
            }
        }
        if let Some(id) = &s.id {
            if !seen_step_ids.insert(id.as_str()) {
                errs.push(ValidationError::DuplicateStepId {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    id: id.clone(),
                });
            }
            step_outputs.insert(id.as_str(), effective_outputs(s, outputs_for));
        }
    }

    let scope = PathScope {
        c,
        ch,
        step_outputs: &step_outputs,
        setup_outputs,
        inputs,
        inherited,
    };
    for assertion in &ch.assertions {
        assertion.walk_exprs(|expr_str| {
            let raw = expr_str.raw.as_str();
            walk_checkable_paths(&expr_str.parsed, &mut |p| {
                check_path(&scope, p, raw, &RefSite::Assertion, errs);
            });
        });
    }

    // Beyond assertions, a `$...` reference inside a step's `with:`
    // payload is the most common place an author writes one — and was
    // historically unscanned, so a typo'd or undeclared ref reached
    // the action as a literal `$...` string (#134). Walk every string
    // scalar in the (untyped) `with:` tree and resolve its references
    // against the same scope.
    for (idx, s) in ch.steps.iter().enumerate() {
        let site = RefSite::StepWith {
            step: step_label(s, idx),
        };
        walk_with_refs(&s.with, &mut |expr, raw| {
            walk_checkable_paths(expr, &mut |p| {
                check_path(&scope, p, raw, &site, errs);
            });
        });
    }
}

/// A step's human label for diagnostics: its `id` when declared, else
/// `step <index>` so a ref inside an anonymous step is still locatable.
fn step_label(s: &Step, idx: usize) -> String {
    s.id.clone().unwrap_or_else(|| format!("step {idx}"))
}

/// Walk a step's untyped `with:` tree, invoking `visit(expr, raw)` for
/// every string scalar that parses as a *substitutable* expression
/// (`Expr::Path | Expr::Call`) — matching the runtime's
/// `is_substitutable_expr`. Strings that don't lead with `$`, don't
/// parse, or are non-substitutable (comparisons, literals) are not
/// cross-references and are skipped — conservatively, so we never
/// reject a value the runtime would pass through untouched.
fn walk_with_refs<F: FnMut(&Expr, &str)>(with: &serde_yml::Value, visit: &mut F) {
    match with {
        serde_yml::Value::String(s) => {
            if !s.trim_start().starts_with('$') {
                return;
            }
            if let Ok(expr) = crate::expr::parse(s)
                && matches!(expr, Expr::Path(_) | Expr::Call { .. })
            {
                visit(&expr, s);
            }
        }
        serde_yml::Value::Sequence(seq) => {
            for v in seq {
                walk_with_refs(v, visit);
            }
        }
        serde_yml::Value::Mapping(map) => {
            for (_k, v) in map {
                walk_with_refs(v, visit);
            }
        }
        _ => {}
    }
}

/// Walk every checkable `Path` in `expr`, with the `default()`
/// carve-out: the first argument of a `$runtime.default(value,
/// fallback)` call is the author's explicit "may be absent" escape
/// hatch (see `eval.rs`'s `default` builtin), so its paths are NOT
/// visited — a missing ref there is the feature, not an error. Every
/// other position is walked normally. Used for both assertions and
/// `with:` so the carve-out is consistent across sites.
fn walk_checkable_paths<F: FnMut(&Path)>(expr: &Expr, visit: &mut F) {
    match expr {
        Expr::Lit(_) => {}
        Expr::Path(p) => visit(p),
        Expr::Call { path, args } => {
            visit(path);
            let skip_first = is_default_call(path);
            for (i, a) in args.iter().enumerate() {
                if skip_first && i == 0 {
                    continue;
                }
                walk_checkable_paths(a, visit);
            }
        }
        Expr::BinOp { lhs, rhs, .. } => {
            walk_checkable_paths(lhs, visit);
            walk_checkable_paths(rhs, visit);
        }
        Expr::UnaryOp { expr, .. } => walk_checkable_paths(expr, visit),
    }
}

/// `$runtime.default(...)` — the one builtin whose first argument
/// tolerates a missing reference.
fn is_default_call(path: &Path) -> bool {
    path.root == PathRoot::Runtime && path.segments().len() == 1 && path.segments()[0] == "default"
}

fn check_path(
    scope: &PathScope<'_>,
    path: &Path,
    raw: &str,
    site: &RefSite,
    errs: &mut Vec<ValidationError>,
) {
    let PathScope {
        c,
        ch,
        step_outputs,
        setup_outputs,
        inputs,
        inherited,
    } = *scope;
    match path.root {
        PathRoot::Steps => {
            let segs = path.segments();
            // Leading `$steps.<step_id>.outputs.<output>`; any segments
            // past index 2 navigate into a structured output and are the
            // runtime evaluator's job to resolve, not the schema's (the
            // schema can't know the runtime JSON shape).
            if segs.len() < 3 || segs[1] != "outputs" {
                errs.push(ValidationError::MalformedStepRef {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    raw: raw.to_string(),
                    site: site.clone(),
                });
                return;
            }
            let step_id = segs[0].as_str();
            let output_name = segs[2].as_str();
            match step_outputs.get(step_id) {
                None => errs.push(ValidationError::UnresolvedStepRef {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    step: step_id.to_string(),
                    raw: raw.to_string(),
                    site: site.clone(),
                }),
                Some(outputs) => {
                    if !outputs.contains(output_name) {
                        errs.push(ValidationError::UnresolvedStepOutput {
                            criterion: c.id.clone(),
                            check: ch.id.clone(),
                            step: step_id.to_string(),
                            output: output_name.to_string(),
                            raw: raw.to_string(),
                            site: site.clone(),
                        });
                    }
                }
            }
        }
        PathRoot::Setup => {
            let segs = path.segments();
            // Leading `$setup.<step_id>.outputs.<output>` — same shape as
            // `$steps`; deeper segments navigate the value at runtime.
            if segs.len() < 3 || segs[1] != "outputs" {
                errs.push(ValidationError::MalformedSetupRef {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    raw: raw.to_string(),
                    site: site.clone(),
                });
                return;
            }
            let step_id = segs[0].as_str();
            let output_name = segs[2].as_str();
            match setup_outputs.get(step_id) {
                None => errs.push(ValidationError::UnresolvedSetupStepRef {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    step: step_id.to_string(),
                    raw: raw.to_string(),
                    site: site.clone(),
                }),
                Some(outputs) => {
                    if !outputs.contains(output_name) {
                        errs.push(ValidationError::UnresolvedSetupStepOutput {
                            criterion: c.id.clone(),
                            check: ch.id.clone(),
                            step: step_id.to_string(),
                            output: output_name.to_string(),
                            raw: raw.to_string(),
                            site: site.clone(),
                        });
                    }
                }
            }
        }
        PathRoot::Inputs => {
            let segs = path.segments();
            // Leading `$inputs.<name>`; deeper segments navigate into a
            // declared `object` / `array` input at runtime.
            if segs.is_empty() {
                errs.push(ValidationError::MalformedInputRef {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    raw: raw.to_string(),
                    site: site.clone(),
                });
                return;
            }
            let name = segs[0].as_str();
            // `$inputs.<name>` resolves against `inputs:` ∪ `inherits:`
            // (spec #135): an inherited name is bound by the parent
            // manifest's chain at run time, so it is not a typo here.
            if !inputs.contains_key(name) && !inherited.contains(name) {
                errs.push(ValidationError::UnresolvedInputRef {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    input: name.to_string(),
                    raw: raw.to_string(),
                    site: site.clone(),
                });
            }
        }
        PathRoot::Env | PathRoot::Runtime => {
            // `$env` and `$runtime` are open catalogs at the schema
            // layer; the runtime spec validates the whitelist /
            // helper set.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::VerificationDefinition;

    fn parse(y: &str) -> VerificationDefinition {
        VerificationDefinition::from_yaml_str(y).expect("parse")
    }

    // ---- implicit output inference (spec #267) ----

    /// Stand-in for the CLI's `contract_outputs`: mimics the api/call
    /// contract (`status` / `body` / `body_text`), empty otherwise.
    fn api_call_outputs(uses: &str) -> Vec<String> {
        if uses == "api/call" {
            ["status", "body", "body_text"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// A step that binds NO `outputs:` yet asserts over `status`.
    const TERSE_VD: &str = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        description: a
        steps:
          - id: home
            uses: api/call
            with: { method: GET, url: https://example.com }
        assertions:
          - $steps.home.outputs.status == 200
"#;

    #[test]
    fn contract_output_reference_without_binding_is_ok_with_inference() {
        // `$steps.home.outputs.status` resolves against the api/call
        // contract though the step declares no `outputs:` (spec #267).
        let v = parse(TERSE_VD);
        assert!(
            validate_with_contract_outputs(&v, &|u| api_call_outputs(u)).is_ok(),
            "{:?}",
            validate_with_contract_outputs(&v, &|u| api_call_outputs(u))
        );
    }

    #[test]
    fn contract_output_reference_without_binding_fails_under_strict_validate() {
        // Backward compat: `validate` (no contract knowledge) still
        // requires the output be bound in `outputs:`.
        let v = parse(TERSE_VD);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvedStepOutput { output, step, .. }
                    if output == "status" && step == "home"
            )),
            "{errs:?}"
        );
    }

    #[test]
    fn undeclared_output_still_fails_with_inference() {
        // Inference widens to contract outputs, not to *any* name.
        let y = TERSE_VD.replace("outputs.status", "outputs.nope");
        let v = parse(&y);
        let errs = validate_with_contract_outputs(&v, &|u| api_call_outputs(u)).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvedStepOutput { output, .. } if output == "nope"
            )),
            "{errs:?}"
        );
    }

    #[test]
    fn explicit_binding_and_inference_coexist() {
        // An authored rename alias still resolves alongside inferred
        // contract outputs — the union, not a replacement.
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        description: a
        steps:
          - id: home
            uses: api/call
            with: { method: GET, url: https://example.com }
            outputs: { code: status }
        assertions:
          - $steps.home.outputs.code == 200
          - $steps.home.outputs.body_text == "x"
"#;
        let v = parse(y);
        assert!(
            validate_with_contract_outputs(&v, &|u| api_call_outputs(u)).is_ok(),
            "{:?}",
            validate_with_contract_outputs(&v, &|u| api_call_outputs(u))
        );
    }

    #[test]
    fn empty_criteria_fails() {
        let v = parse("verification: x\ncriteria: []\n");
        let errs = validate(&v).unwrap_err();
        assert!(matches!(errs[0], ValidationError::NoCriteria));
    }

    #[test]
    fn check_with_no_assertions_and_no_steps_fails() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::NothingToJudge { criterion, check }
                    if criterion == "AC-1" && check == "AC-1.1"
            )),
            "{errs:?}"
        );
    }

    #[test]
    fn check_with_steps_but_no_assertions_is_accepted_at_schema_layer() {
        // Whether a step actually judges (spec #253) is a catalog
        // question — the contract-aware CLI layer owns it.
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/assert-url
            with: { matches: "/login" }
"#;
        let v = parse(y);
        assert!(validate(&v).is_ok());
    }

    #[test]
    fn duplicate_criterion_id_fails() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions: [$inputs.x]
  - id: AC-1
    description: b
    checks:
      - id: AC-1.1
        assertions: [$inputs.x]
inputs:
  x: { type: string }
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::DuplicateCriterionId { id } if id == "AC-1"))
        );
    }

    #[test]
    fn duplicate_check_id_fails() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions: [$inputs.x]
      - id: AC-1.1
        assertions: [$inputs.x]
inputs:
  x: { type: string }
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::DuplicateCheckId { .. }))
        );
    }

    #[test]
    fn duplicate_step_id_fails() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - id: a
            uses: ui/click
            with: {}
          - id: a
            uses: ui/click
            with: {}
        assertions: [$inputs.x]
inputs:
  x: { type: string }
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::DuplicateStepId { .. }))
        );
    }

    #[test]
    fn unresolved_step_ref_fails() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps: []
        assertions:
          - $steps.nope.outputs.foo == 1
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(errs.iter().any(
            |e| matches!(e, ValidationError::UnresolvedStepRef { step, .. } if step == "nope")
        ));
    }

    #[test]
    fn unresolved_step_output_fails() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - id: api
            uses: api/observe
            with: {}
            outputs:
              status: response.status
        assertions:
          - $steps.api.outputs.body == 1
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(e, ValidationError::UnresolvedStepOutput { output, .. } if output == "body"))
        );
    }

    #[test]
    fn unresolved_input_ref_fails() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.nope == 1
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(errs.iter().any(
            |e| matches!(e, ValidationError::UnresolvedInputRef { input, .. } if input == "nope")
        ));
    }

    #[test]
    fn malformed_step_ref_fails() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps: []
        assertions:
          - exists: $steps.foo
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::MalformedStepRef { .. }))
        );
    }

    #[test]
    fn step_ref_with_nav_segments_validates() {
        // Deeper navigation into a declared output (#104) — the schema
        // resolves the `$steps.api.outputs.body` address and leaves
        // `.extra` / `[0]` for the runtime evaluator.
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - id: api
            uses: api/observe
            with: {}
            outputs:
              body: response.body
        assertions:
          - exists: $steps.api.outputs.body.app.id
          - exists: $steps.api.outputs.body.items[0]
"#;
        let v = parse(y);
        assert!(validate(&v).is_ok());
    }

    #[test]
    fn step_ref_unknown_output_still_fails_under_nav() {
        // The leading address is still validated: an undeclared output
        // is an error even when followed by navigation segments.
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - id: api
            uses: api/observe
            with: {}
            outputs:
              body: response.body
        assertions:
          - exists: $steps.api.outputs.nope.app
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::UnresolvedStepOutput { .. }))
        );
    }

    #[test]
    fn input_ref_with_nav_segments_validates() {
        let y = r#"
verification: x
inputs:
  cfg:
    type: object
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - exists: $inputs.cfg.db.host
"#;
        let v = parse(y);
        assert!(validate(&v).is_ok());
    }

    #[test]
    fn env_paths_pass() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - exists: $env.DATABASE_URL
"#;
        let v = parse(y);
        validate(&v).expect("$env should not fail");
    }

    #[test]
    fn runtime_paths_pass() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - exists: $runtime.uuid()
"#;
        let v = parse(y);
        validate(&v).expect("$runtime should not fail");
    }

    #[test]
    fn duplicate_setup_step_id_fails() {
        let y = r#"
verification: x
setup:
  - id: warm
    uses: ui/navigate
    with: {}
  - id: warm
    uses: ui/navigate
    with: {}
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::DuplicateSetupStepId { .. })),
            "got: {errs:?}"
        );
    }

    #[test]
    fn unresolved_setup_step_ref_fails() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - $setup.nope.outputs.x == 1
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvedSetupStepRef { step, .. } if step == "nope"
            )),
            "got: {errs:?}"
        );
    }

    #[test]
    fn unresolved_setup_step_output_fails() {
        let y = r#"
verification: x
setup:
  - id: warm
    uses: ui/navigate
    with: {}
    outputs:
      landed_at: page.url
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - $setup.warm.outputs.missing == 1
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvedSetupStepOutput { output, .. } if output == "missing"
            )),
            "got: {errs:?}"
        );
    }

    #[test]
    fn malformed_setup_ref_fails() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - exists: $setup.foo
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::MalformedSetupRef { .. })),
            "got: {errs:?}"
        );
    }

    #[test]
    fn good_setup_block_passes() {
        let y = r#"
verification: ok
setup:
  - id: warm
    uses: ui/navigate
    with: {}
    outputs:
      landed_at: page.url
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - $setup.warm.outputs.landed_at == "x"
"#;
        let v = parse(y);
        validate(&v).expect("should validate");
    }

    #[test]
    fn matching_defaults_validate_for_every_type() {
        let y = r#"
verification: ok
inputs:
  s: { type: string,  default: "hi" }
  i: { type: integer, default: 3 }
  n: { type: number,  default: 0.85 }
  b: { type: boolean, default: true }
  a: { type: array,   default: ["x", "y"] }
  o: { type: object,  default: { k: 1 } }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;
        let v = parse(y);
        validate(&v).expect("all defaults match");
    }

    #[test]
    fn integer_default_is_valid_for_number_type() {
        let y = r#"
verification: ok
inputs:
  threshold: { type: number, default: 1 }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;
        let v = parse(y);
        validate(&v).expect("integer default under `number` should validate");
    }

    #[test]
    fn fractional_default_under_integer_type_fails() {
        let y = r#"
verification: x
inputs:
  count: { type: integer, default: 0.5 }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::InputDefaultTypeMismatch { input, declared, actual }
                    if input == "count" && *declared == InputType::Integer && actual == "number"
            )),
            "expected InputDefaultTypeMismatch, got {errs:?}"
        );
    }

    #[test]
    fn authored_output_under_capture_prefix_is_rejected() {
        // The `capture/` namespace is reserved for runner-emitted
        // failure evidence (spec #202); an authored output can't
        // masquerade as a capture.
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - id: s
            uses: ui/navigate
            with: { url: http://x }
            outputs:
              capture/screenshot: satisfied
        assertions: ["true"]
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::ReservedOutputPrefix { criterion, check, name, .. }
                    if criterion == "AC-1" && check == "AC-1.1" && name == "capture/screenshot"
            )),
            "expected ReservedOutputPrefix, got {errs:?}"
        );
    }

    #[test]
    fn ordinary_output_name_is_allowed() {
        // A normal alias with no reserved prefix validates fine — the
        // guard must not over-reject.
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - id: s
            uses: ui/assert-element
            with: { locator: { role: button, name: Go }, expected: visible }
            outputs:
              satisfied: satisfied
        assertions:
          - $steps.s.outputs.satisfied == true
"#;
        let v = parse(y);
        validate(&v).expect("validate");
    }

    #[test]
    fn yaml_null_default_collapses_to_no_default() {
        let y = r#"
verification: x
inputs:
  name: { type: string, default: null }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;
        let v = parse(y);
        validate(&v).expect("validate");
        assert!(v.inputs["name"].default.is_none());
    }

    #[test]
    fn tagged_default_classifies_by_inner_shape() {
        let y = r#"
verification: x
inputs:
  name: { type: string, default: !!str 3 }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;
        let v = parse(y);
        validate(&v).expect("tagged string default matches `type: string`");
    }

    #[test]
    fn string_default_under_integer_fails() {
        let y = r#"
verification: x
inputs:
  count: { type: integer, default: "nope" }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::InputDefaultTypeMismatch { input, .. } if input == "count"
            )),
            "expected mismatch on `count`, got {errs:?}"
        );
    }

    #[test]
    fn with_ref_to_undeclared_input_fails() {
        // A step `with: { url: $inputs.undeclared }` — historically
        // unscanned (#134) — now resolves against `inputs:` and fails.
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - uses: api/call
            with:
              url: $inputs.undeclared
        assertions: ["true"]
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvedInputRef { input, site, .. }
                    if input == "undeclared"
                        && matches!(site, RefSite::StepWith { .. })
            )),
            "got: {errs:?}"
        );
    }

    #[test]
    fn with_ref_names_the_step_in_the_message() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - id: login
            uses: api/call
            with:
              url: $inputs.undeclared
        assertions: ["true"]
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        let msg = errs
            .iter()
            .find(|e| matches!(e, ValidationError::UnresolvedInputRef { .. }))
            .map(|e| e.to_string())
            .unwrap();
        assert!(msg.contains("step `login` with:"), "got: {msg}");
    }

    #[test]
    fn with_ref_to_earlier_step_output_passes() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - id: login
            uses: api/observe
            with: {}
            outputs:
              token: response.body.token
          - uses: api/call
            with:
              auth: $steps.login.outputs.token
        assertions: ["true"]
"#;
        let v = parse(y);
        validate(&v).expect("earlier-step output ref in with: resolves");
    }

    #[test]
    fn with_ref_typo_output_fails_with_step_output() {
        // The worked example: a typo'd output name inside `with:`.
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - id: login
            uses: api/observe
            with: {}
            outputs:
              token: response.body.token
          - uses: api/call
            with:
              auth: $steps.login.outputs.toekn
        assertions: ["true"]
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvedStepOutput { output, step, .. }
                    if output == "toekn" && step == "login"
            )),
            "got: {errs:?}"
        );
    }

    #[test]
    fn with_default_carveout_allows_missing_input() {
        // `$runtime.default($inputs.maybe, "x")` inside with: — the
        // first arg may be absent; validate must not flag it.
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - uses: api/call
            with:
              url: $runtime.default($inputs.maybe, "fallback")
        assertions: ["true"]
"#;
        let v = parse(y);
        validate(&v).expect("default()'s first arg is missing-tolerant in with:");
    }

    #[test]
    fn assertion_default_carveout_allows_missing_input() {
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - $runtime.default($inputs.maybe, "x") == "x"
"#;
        let v = parse(y);
        validate(&v).expect("default()'s first arg is missing-tolerant in assertions");
    }

    #[test]
    fn default_second_arg_still_strict() {
        // Only the FIRST arg is carved out; a missing ref in the
        // fallback position is still an error.
        let y = r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - uses: api/call
            with:
              url: $runtime.default("x", $inputs.also_missing)
        assertions: ["true"]
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvedInputRef { input, .. } if input == "also_missing"
            )),
            "got: {errs:?}"
        );
    }

    #[test]
    fn inherited_input_ref_resolves() {
        // `$inputs.base_url` with `base_url` listed under `inherits:`
        // (and no local `inputs:` for it) is not a typo (spec #135).
        let y = r#"
verification: leaf
inherits:
  - base_url
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.base_url == "x"
"#;
        let v = parse(y);
        validate(&v).expect("inherited input ref resolves");
    }

    #[test]
    fn inherited_input_also_declared_fails() {
        let y = r#"
verification: leaf
inputs:
  base_url: { type: string }
inherits:
  - base_url
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.base_url == "x"
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::InheritedInputAlsoDeclared { name } if name == "base_url"
            )),
            "got: {errs:?}"
        );
    }

    #[test]
    fn empty_inherited_name_fails() {
        let y = r#"
verification: leaf
inherits:
  - ""
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::EmptyInheritedName { .. })),
            "got: {errs:?}"
        );
    }

    #[test]
    fn bare_input_ref_not_inherited_still_typo() {
        // A `$inputs.x` neither in `inputs:` nor `inherits:` is still a
        // typo error (#134), distinct from the inherited case.
        let y = r#"
verification: leaf
inherits:
  - base_url
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.typo == 1
"#;
        let v = parse(y);
        let errs = validate(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvedInputRef { input, .. } if input == "typo"
            )),
            "got: {errs:?}"
        );
    }

    #[test]
    fn good_definition_passes() {
        let y = r#"
verification: ok
inputs:
  workspace_name: { type: string }
criteria:
  - id: AC-1
    description: trivial
    checks:
      - id: AC-1.1
        steps:
          - id: api
            uses: api/observe
            with: { method: POST }
            outputs:
              status: response.status
        assertions:
          - $steps.api.outputs.status == 200
          - type_check: { value: $steps.api.outputs.status, is: integer }
          - $inputs.workspace_name == "x"
"#;
        let v = parse(y);
        validate(&v).expect("should validate");
    }
}
