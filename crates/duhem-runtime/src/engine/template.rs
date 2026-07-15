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
use duhem_schema::expr::Path;

use crate::engine::context::value_to_yml;
use crate::eval::{EvalContext, eval_to_value};

/// A `with:` value that failed to resolve, pinpointed to the specific
/// `$...` sub-expression the caller can name in the engine error (#238).
///
/// `reference` is the smallest offending reference — for a bare
/// `$inputs.x` it is that reference; for a `$runtime.format(...)`-style
/// call it is the first *argument* that didn't resolve (e.g.
/// `$steps.create.outputs.body.data._id`), so the error no longer
/// misattributes the failure to the whole call. `context`, when the
/// reference is a sub-part, carries the enclosing expression's source so
/// the message can show "…in `with:` (evaluating `$runtime.format(...)`)".
#[derive(Debug)]
pub struct UnresolvedWith {
    pub reference: String,
    pub context: Option<String>,
}

/// Outcome of resolving one `with:` string slot.
enum Resolution {
    /// The string was a `$`-leading substitutable expr that evaluated
    /// to a scalar — splice it in.
    Replace(serde_yml::Value),
    /// The string was not a substitutable reference (no leading `$`,
    /// or parses as an assertion-shaped expr) — pass through unchanged.
    Passthrough,
    /// The string WAS a bare `$...` reference (or a call over one) but
    /// evaluation failed — a hard error. No action input may carry a
    /// literal `$...` string, so we surface the unresolved reference
    /// (#134), pinpointed to the failing sub-expression (#238). A
    /// `default(...)` call evaluates successfully (yields its fallback)
    /// and so never reaches here.
    Unresolved(UnresolvedWith),
}

/// Recursively walk `with`, substituting any string value that parses
/// as an `Expr` whose evaluation produces a scalar `Value`. Mutates
/// in place. A `$...` reference that fails to evaluate is a hard error:
/// the returned [`UnresolvedWith`] pinpoints the offending
/// sub-reference so the caller can name it alongside the step (#134,
/// #238).
pub fn substitute_with(
    with: &mut serde_yml::Value,
    ctx: &dyn EvalContext,
) -> Result<(), UnresolvedWith> {
    match with {
        serde_yml::Value::String(s) => match try_resolve(s, ctx) {
            Resolution::Replace(replacement) => {
                *with = replacement;
                Ok(())
            }
            Resolution::Passthrough => Ok(()),
            Resolution::Unresolved(u) => Err(u),
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
        Err(_) => Resolution::Unresolved(pinpoint(&expr, s, ctx)),
    }
}

fn is_substitutable_expr(e: &Expr) -> bool {
    matches!(e, Expr::Path(_) | Expr::Call { .. })
}

/// Pinpoint the specific `$...` sub-reference of `expr` (whose overall
/// evaluation just failed) that did not resolve, so the engine error
/// names the missing value rather than the enclosing call (#238).
///
/// - A bare path is its own culprit.
/// - A `$runtime.format(...)`-style call walks to the first argument
///   that fails to evaluate and recurses, so a missing
///   `$steps.create.outputs.body.data._id` argument is named — not the
///   whole `format(...)`. The enclosing expression's source is kept as
///   `context`.
/// - When no single sub-path is at fault (e.g. a `format` string with
///   the wrong number of `{}`), the whole expression is the reference.
fn pinpoint(expr: &Expr, raw: &str, ctx: &dyn EvalContext) -> UnresolvedWith {
    let raw = raw.trim();
    match culprit(expr, ctx) {
        Some(p) => {
            let reference = render_path(p);
            // Only add context when the culprit is a *sub*-expression;
            // for a bare path the reference already is the whole thing.
            let context = (reference != raw).then(|| raw.to_string());
            UnresolvedWith { reference, context }
        }
        None => UnresolvedWith {
            reference: raw.to_string(),
            context: None,
        },
    }
}

/// The first path within `expr` that fails to evaluate under `ctx`,
/// descending into call arguments. `None` when the failure isn't
/// attributable to a single unresolved path (e.g. bad `format` arity).
fn culprit<'a>(expr: &'a Expr, ctx: &dyn EvalContext) -> Option<&'a Path> {
    match expr {
        Expr::Path(p) => Some(p),
        Expr::Call { args, .. } => args
            .iter()
            .find(|arg| eval_to_value(arg, ctx).is_err())
            .and_then(|arg| culprit(arg, ctx)),
        _ => None,
    }
}

/// Render a parsed [`Path`] back to its `$<root>.<seg>...` source form.
/// Digit-only segments are array indices (`[0]`); everything else is a
/// dotted key — matching the parser's lowering and `eval`'s `nav_path`.
fn render_path(p: &Path) -> String {
    let mut out = String::from("$");
    out.push_str(p.root.as_str());
    for seg in &p.segments {
        if !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit()) {
            out.push('[');
            out.push_str(seg);
            out.push(']');
        } else {
            out.push('.');
            out.push_str(seg);
        }
    }
    out
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
        // A bare ref is its own culprit — no enclosing context.
        assert_eq!(err.reference, "$inputs.unset");
        assert_eq!(err.context, None);
    }

    #[test]
    fn format_arg_pinpoints_the_missing_sub_reference() {
        // #238: a `$runtime.format(...)` whose ARGUMENT is missing must
        // name that argument, not blame the whole call. The first arg
        // resolves; the second (`$steps.gone…`) does not, so the error
        // points at it with the call as context.
        let run = run_with(&[("base", Value::Str("http://x".into()))]);
        let ctx = RunContext::new(&run);
        let mut with: serde_yml::Value = serde_yml::from_str(
            r#"{ url: '$runtime.format("{}/{}", $inputs.base, $steps.gone.outputs.body.id)' }"#,
        )
        .unwrap();
        let err = substitute_with(&mut with, &ctx).unwrap_err();
        assert_eq!(err.reference, "$steps.gone.outputs.body.id");
        assert_eq!(
            err.context.as_deref(),
            Some(r#"$runtime.format("{}/{}", $inputs.base, $steps.gone.outputs.body.id)"#)
        );
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
