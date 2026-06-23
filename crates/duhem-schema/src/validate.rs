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
use crate::expr::{Path, PathRoot};
use crate::step::Step;
use crate::verification::{InputDecl, InputType, VerificationDefinition};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("verification has no criteria")]
    NoCriteria,

    #[error("duplicate criterion id `{id}`")]
    DuplicateCriterionId { id: String },

    #[error("criterion `{criterion}`: duplicate check id `{id}`")]
    DuplicateCheckId { criterion: String, id: String },

    #[error("criterion `{criterion}` / check `{check}`: duplicate step id `{id}`")]
    DuplicateStepId {
        criterion: String,
        check: String,
        id: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: assertion `{raw}` references undeclared step `{step}`"
    )]
    UnresolvedStepRef {
        criterion: String,
        check: String,
        step: String,
        raw: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: assertion `{raw}` references undeclared output `{output}` on step `{step}`"
    )]
    UnresolvedStepOutput {
        criterion: String,
        check: String,
        step: String,
        output: String,
        raw: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: assertion `{raw}` references undeclared input `{input}`"
    )]
    UnresolvedInputRef {
        criterion: String,
        check: String,
        input: String,
        raw: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: malformed `$steps` reference `{raw}` (expected `$steps.<step_id>.outputs.<output>`)"
    )]
    MalformedStepRef {
        criterion: String,
        check: String,
        raw: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: malformed `$inputs` reference `{raw}` (expected `$inputs.<name>`)"
    )]
    MalformedInputRef {
        criterion: String,
        check: String,
        raw: String,
    },

    #[error("setup: duplicate step id `{id}`")]
    DuplicateSetupStepId { id: String },

    #[error(
        "criterion `{criterion}` / check `{check}`: assertion `{raw}` references undeclared setup step `{step}`"
    )]
    UnresolvedSetupStepRef {
        criterion: String,
        check: String,
        step: String,
        raw: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: assertion `{raw}` references undeclared output `{output}` on setup step `{step}`"
    )]
    UnresolvedSetupStepOutput {
        criterion: String,
        check: String,
        step: String,
        output: String,
        raw: String,
    },

    #[error(
        "criterion `{criterion}` / check `{check}`: malformed `$setup` reference `{raw}` (expected `$setup.<step_id>.outputs.<output>`)"
    )]
    MalformedSetupRef {
        criterion: String,
        check: String,
        raw: String,
    },

    #[error(
        "input `{input}`: default value type `{actual}` does not match declared type `{declared}`"
    )]
    InputDefaultTypeMismatch {
        input: String,
        declared: InputType,
        actual: String,
    },
}

/// Run every structural rule. Always reports as many errors as
/// possible — the goal is one round-trip from "save the file" to "see
/// the punch list".
pub fn validate(v: &VerificationDefinition) -> Result<(), Vec<ValidationError>> {
    let mut errs = Vec::new();

    if v.criteria.is_empty() {
        errs.push(ValidationError::NoCriteria);
    }

    let setup_outputs = collect_setup_outputs(&v.setup, &mut errs);

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

    let mut seen_criteria: HashSet<&str> = HashSet::new();
    for c in &v.criteria {
        if !seen_criteria.insert(c.id.as_str()) {
            errs.push(ValidationError::DuplicateCriterionId { id: c.id.clone() });
        }
        validate_criterion(c, &v.inputs, &setup_outputs, &mut errs);
    }

    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

/// Walk the run-level `setup:` block. Enforces id-uniqueness and
/// returns the map of `step_id → declared outputs` so per-check
/// assertion validation can resolve `$setup.<id>.outputs.<name>`
/// references.
fn collect_setup_outputs<'a>(
    setup: &'a [Step],
    errs: &mut Vec<ValidationError>,
) -> HashMap<&'a str, &'a BTreeMap<String, String>> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut outputs: HashMap<&str, &BTreeMap<String, String>> = HashMap::new();
    for s in setup {
        if let Some(id) = &s.id {
            if !seen.insert(id.as_str()) {
                errs.push(ValidationError::DuplicateSetupStepId { id: id.clone() });
            }
            outputs.insert(id.as_str(), &s.outputs);
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
    setup_outputs: &HashMap<&str, &BTreeMap<String, String>>,
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
        validate_check(c, ch, inputs, setup_outputs, errs);
    }
}

/// Per-check view of resolvable references — bundled to keep
/// `check_path`'s arity within clippy's `too_many_arguments` lint.
struct PathScope<'a> {
    c: &'a Criterion,
    ch: &'a Check,
    step_outputs: &'a HashMap<&'a str, &'a BTreeMap<String, String>>,
    setup_outputs: &'a HashMap<&'a str, &'a BTreeMap<String, String>>,
    inputs: &'a BTreeMap<String, InputDecl>,
}

fn validate_check(
    c: &Criterion,
    ch: &Check,
    inputs: &BTreeMap<String, InputDecl>,
    setup_outputs: &HashMap<&str, &BTreeMap<String, String>>,
    errs: &mut Vec<ValidationError>,
) {
    let mut step_outputs: HashMap<&str, &BTreeMap<String, String>> = HashMap::new();
    let mut seen_step_ids: HashSet<&str> = HashSet::new();

    for s in &ch.steps {
        if let Some(id) = &s.id {
            if !seen_step_ids.insert(id.as_str()) {
                errs.push(ValidationError::DuplicateStepId {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    id: id.clone(),
                });
            }
            step_outputs.insert(id.as_str(), &s.outputs);
        }
    }

    let scope = PathScope {
        c,
        ch,
        step_outputs: &step_outputs,
        setup_outputs,
        inputs,
    };
    for assertion in &ch.assertions {
        assertion.walk_exprs(|expr_str| {
            let raw = expr_str.raw.as_str();
            expr_str.parsed.walk_paths(|p| {
                check_path(&scope, p, raw, errs);
            });
        });
    }
}

fn check_path(scope: &PathScope<'_>, path: &Path, raw: &str, errs: &mut Vec<ValidationError>) {
    let PathScope {
        c,
        ch,
        step_outputs,
        setup_outputs,
        inputs,
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
                }),
                Some(outputs) => {
                    if !outputs.contains_key(output_name) {
                        errs.push(ValidationError::UnresolvedStepOutput {
                            criterion: c.id.clone(),
                            check: ch.id.clone(),
                            step: step_id.to_string(),
                            output: output_name.to_string(),
                            raw: raw.to_string(),
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
                }),
                Some(outputs) => {
                    if !outputs.contains_key(output_name) {
                        errs.push(ValidationError::UnresolvedSetupStepOutput {
                            criterion: c.id.clone(),
                            check: ch.id.clone(),
                            step: step_id.to_string(),
                            output: output_name.to_string(),
                            raw: raw.to_string(),
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
                });
                return;
            }
            let name = segs[0].as_str();
            if !inputs.contains_key(name) {
                errs.push(ValidationError::UnresolvedInputRef {
                    criterion: c.id.clone(),
                    check: ch.id.clone(),
                    input: name.to_string(),
                    raw: raw.to_string(),
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

    #[test]
    fn empty_criteria_fails() {
        let v = parse("verification: x\ncriteria: []\n");
        let errs = validate(&v).unwrap_err();
        assert!(matches!(errs[0], ValidationError::NoCriteria));
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
