//! `Step.with` template substitution.
//!
//! The on-the-wire `Step.with` carries opaque YAML that the action's
//! `With` schema deserializes. To make a fixture like
//! `with: { url: $inputs.fixture_url }` actually executable, we
//! resolve any string value that parses as an `Expr::Path` against
//! the current `EvalContext` and substitute the evaluated scalar in
//! place. Strings that don't start with `$`, or that don't parse as
//! a path/runtime call, pass through unchanged — the substitution
//! is conservative.
//!
//! This is intentionally narrower than full string interpolation
//! (e.g. `"prefix-{{ $inputs.x }}-suffix"`). The spec on issue #15
//! calls out "no new on-the-wire surface"; this is the minimum
//! that makes the worked-example fixture from #12 executable.

use duhem_schema::Expr;

use crate::engine::context::value_to_yml;
use crate::eval::{EvalContext, eval_to_value};

/// Outcome of resolving one `with:` string slot.
enum Resolution {
    /// The string was a `$`-leading substitutable expr that evaluated
    /// to a scalar — splice it in.
    Replace(serde_yml::Value),
    /// The string was not a substitutable reference (no leading `$`,
    /// or parses as an assertion-shaped expr) — pass through unchanged.
    Passthrough,
    /// The string WAS a bare `$...` reference but evaluation failed —
    /// a hard error. No action input may carry a literal `$...`
    /// string, so we surface the unresolved reference (#134). A
    /// `default(...)` call evaluates successfully (yields its
    /// fallback) and so never reaches here.
    Unresolved,
}

/// Recursively walk `with`, substituting any string value that parses
/// as an `Expr` whose evaluation produces a scalar `Value`. Mutates
/// in place. A bare `$...` reference that fails to evaluate is a hard
/// error: `Err(raw)` carries the offending reference's source so the
/// caller can name it alongside the step (#134).
pub fn substitute_with(with: &mut serde_yml::Value, ctx: &dyn EvalContext) -> Result<(), String> {
    match with {
        serde_yml::Value::String(s) => match try_resolve(s, ctx) {
            Resolution::Replace(replacement) => {
                *with = replacement;
                Ok(())
            }
            Resolution::Passthrough => Ok(()),
            Resolution::Unresolved => Err(s.clone()),
        },
        serde_yml::Value::Sequence(seq) => {
            for v in seq.iter_mut() {
                substitute_with(v, ctx)?;
            }
            Ok(())
        }
        serde_yml::Value::Mapping(map) => {
            for (_k, v) in map.iter_mut() {
                substitute_with(v, ctx)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn try_resolve(s: &str, ctx: &dyn EvalContext) -> Resolution {
    // Only consider strings whose first non-whitespace character is
    // `$`. Otherwise we'd accidentally evaluate plain integer-shaped
    // strings (`"200"`) as Expr literals and substitute them into a
    // String slot. The author intent for `$inputs.X` / `$steps.X` /
    // `$runtime.X()` is unambiguous; everything else stays a string.
    if !s.trim_start().starts_with('$') {
        return Resolution::Passthrough;
    }
    let Ok(expr) = duhem_schema::expr::parse(s) else {
        return Resolution::Passthrough;
    };
    // Allow only path / runtime-call expressions — anything else
    // (boolean ops, comparisons) was clearly authored as an
    // assertion, not as a value to splice in. Authors don't write
    // `(1 == 1)` inside `with:`; if they do, we leave it alone.
    if !is_substitutable_expr(&expr) {
        return Resolution::Passthrough;
    }
    // A bare `$...` reference that fails to evaluate is a hard error,
    // never a pass-through (#134): no action may receive a literal
    // `$...` string. `$runtime.default(value, fallback)` evaluates
    // successfully even when `value` is missing — it yields the
    // fallback — so the carve-out is automatic; we don't special-case
    // it here.
    match eval_to_value(&expr, ctx) {
        Ok(value) => Resolution::Replace(value_to_yml(&value)),
        Err(_) => Resolution::Unresolved,
    }
}

fn is_substitutable_expr(e: &Expr) -> bool {
    matches!(e, Expr::Path(_) | Expr::Call { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::context::{RunContext, RunState};
    use crate::eval::Value;
    use std::collections::BTreeMap;

    fn run_with(inputs: &[(&str, Value)]) -> RunState {
        let mut m = BTreeMap::new();
        for (k, v) in inputs {
            m.insert((*k).into(), v.clone());
        }
        RunState::new(m)
    }

    #[test]
    fn substitutes_input_path_inside_mapping() {
        let run = run_with(&[("url", Value::Str("http://x".into()))]);
        let ctx = RunContext::new(&run);
        let mut with: serde_yml::Value = serde_yml::from_str("url: $inputs.url").unwrap();
        substitute_with(&mut with, &ctx).expect("resolves");
        let map = with.as_mapping().unwrap();
        let url = map.get(serde_yml::Value::String("url".into())).unwrap();
        assert_eq!(url.as_str(), Some("http://x"));
    }

    #[test]
    fn leaves_non_template_strings_alone() {
        let run = run_with(&[]);
        let ctx = RunContext::new(&run);
        let mut with: serde_yml::Value =
            serde_yml::from_str("{ role: button, name: Create }").unwrap();
        let before = with.clone();
        substitute_with(&mut with, &ctx).expect("no refs to resolve");
        assert_eq!(with, before);
    }

    #[test]
    fn bare_missing_ref_is_an_error() {
        // #134: a bare `$...` reference that resolves to nothing is a
        // hard error — never a pass-through. The error carries the
        // offending reference's raw source so the caller can name it.
        let run = run_with(&[]);
        let ctx = RunContext::new(&run);
        let mut with: serde_yml::Value = serde_yml::from_str("{ url: $inputs.unset }").unwrap();
        let err = substitute_with(&mut with, &ctx).unwrap_err();
        assert_eq!(err, "$inputs.unset");
    }

    #[test]
    fn default_with_missing_input_resolves_to_fallback() {
        // The carve-out: `default($inputs.unset, "fallback")` evaluates
        // successfully (yields the fallback), so it is NOT an error.
        let run = run_with(&[]);
        let ctx = RunContext::new(&run);
        let mut with: serde_yml::Value =
            serde_yml::from_str(r#"{ url: '$runtime.default($inputs.unset, "fallback")' }"#)
                .unwrap();
        substitute_with(&mut with, &ctx).expect("default() yields fallback, not an error");
        let map = with.as_mapping().unwrap();
        assert_eq!(
            map.get(serde_yml::Value::String("url".into()))
                .and_then(|v| v.as_str()),
            Some("fallback")
        );
    }
}
