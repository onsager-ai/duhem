//! `Step.with` template substitution.
//!
//! The on-the-wire `Step.with` carries opaque YAML that the action's
//! `With` schema deserializes. To make a fixture like
//! `with: { url: $inputs.fixture_url }` actually executable, we
//! resolve any string value that parses as an `Expr::Path` against
//! the current `EvalContext` and substitute the evaluated scalar in
//! place. Strings that don't start with `$` pass through unchanged;
//! `$`-leading strings are expression-shaped author intent, so a parse
//! failure is a hard error and never reaches the action as a literal.
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

impl UnresolvedWith {
    pub(crate) fn rendered_context(&self) -> String {
        self.context
            .as_deref()
            .map(|context| {
                if context.starts_with("expression parse error:") {
                    format!(" ({context})")
                } else {
                    format!(" (evaluating `{context}`)")
                }
            })
            .unwrap_or_default()
    }
}

/// First whole-string `$pages.<page>.<element>` reference in an
/// authored `with:` tree. Retained beside the resolved locator solely
/// for human failure detail; action dispatch still receives the map.
pub(crate) fn page_reference(with: &serde_yml::Value) -> Option<String> {
    match with {
        serde_yml::Value::String(raw) => match duhem_schema::expr::parse(raw).ok()? {
            Expr::Path(Path {
                root: duhem_schema::PathRoot::Pages,
                segments,
            }) if segments.len() == 2 => Some(raw.trim().to_string()),
            Expr::Call {
                path:
                    Path {
                        root: duhem_schema::PathRoot::Pages,
                        segments,
                    },
                ..
            } if segments.len() == 2 => Some(raw.trim().to_string()),
            _ => None,
        },
        serde_yml::Value::Sequence(values) => values.iter().find_map(page_reference),
        serde_yml::Value::Mapping(values) => values.values().find_map(page_reference),
        _ => None,
    }
}

/// Outcome of resolving one `with:` string slot.
enum Resolution {
    /// The string was a `$`-leading substitutable expr that evaluated
    /// to a scalar — splice it in.
    Replace {
        value: serde_yml::Value,
        nested: NestedResolution,
    },
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

enum NestedResolution {
    None,
    /// Catalog entries may themselves contain `$inputs.*`. Retain the
    /// authored entry so filled arguments are not re-scanned as expressions.
    PageEntry(serde_yml::Value),
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
            Resolution::Replace {
                value: replacement,
                nested,
            } => {
                *with = replacement;
                match nested {
                    NestedResolution::None => Ok(()),
                    NestedResolution::PageEntry(authored) => {
                        resolve_page_entry_nested(&authored, with, ctx)
                    }
                }
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
    let expr = match duhem_schema::expr::parse(s) {
        Ok(expr) => expr,
        Err(error) => {
            return Resolution::Unresolved(UnresolvedWith {
                reference: s.trim().to_string(),
                context: Some(error.to_string()),
            });
        }
    };
    // Allow only path / runtime-call expressions — anything else
    // (boolean ops, comparisons) was clearly authored as an
    // assertion, not as a value to splice in. Authors don't write
    // `(1 == 1)` inside `with:`; if they do, we leave it alone.
    if !is_substitutable_expr(&expr) {
        return Resolution::Passthrough;
    }
    if let Some((path, args)) = page_call(&expr) {
        return match resolve_page_call(path, args, ctx) {
            Ok((value, authored)) => Resolution::Replace {
                value,
                nested: NestedResolution::PageEntry(authored),
            },
            Err(()) => Resolution::Unresolved(pinpoint(&expr, s, ctx)),
        };
    }
    // A bare `$...` reference that fails to evaluate is a hard error,
    // never a pass-through (#134): no action may receive a literal
    // `$...` string. `$runtime.default(value, fallback)` evaluates
    // successfully even when `value` is missing — it yields the
    // fallback — so the carve-out is automatic; we don't special-case
    // it here.
    match eval_to_value(&expr, ctx) {
        Ok(value) => Resolution::Replace {
            value: value_to_yml(&value),
            nested: NestedResolution::None,
        },
        Err(_) => Resolution::Unresolved(pinpoint(&expr, s, ctx)),
    }
}

fn page_call(expr: &Expr) -> Option<(&Path, &[Expr])> {
    match expr {
        Expr::Path(path) if path.root == duhem_schema::PathRoot::Pages => Some((path, &[])),
        Expr::Call { path, args } if path.root == duhem_schema::PathRoot::Pages => {
            Some((path, args))
        }
        _ => None,
    }
}

/// Resolve and fill the whole catalog entry before its authored nested
/// expressions are walked (spec #495). Validation normally guarantees
/// the two-segment shape, escaping, and arity; errors remain hard here
/// for callers that bypass validation.
fn resolve_page_call(
    path: &Path,
    args: &[Expr],
    ctx: &dyn EvalContext,
) -> Result<(serde_yml::Value, serde_yml::Value), ()> {
    let [page, element] = path.segments.as_slice() else {
        return Err(());
    };
    let entry = ctx.page(page, element).cloned().ok_or(())?;
    let authored = value_to_yml(&entry);
    let mut filled = authored.clone();
    let parts = args
        .iter()
        .map(|arg| {
            eval_to_value(arg, ctx)
                .and_then(crate::eval::scalar_to_string)
                .map_err(|_| ())
        })
        .collect::<Result<Vec<_>, _>>()?;
    duhem_schema::page_template::fill_value_placeholders(&mut filled, &parts).map_err(|_| ())?;
    Ok((filled, authored))
}

/// Resolve only expressions that were present in the catalog entry.
/// Strings changed by placeholder filling stay literal, even when an
/// argument value begins with `$` and looks like another expression.
fn resolve_page_entry_nested(
    authored: &serde_yml::Value,
    filled: &mut serde_yml::Value,
    ctx: &dyn EvalContext,
) -> Result<(), UnresolvedWith> {
    match (authored, filled) {
        (serde_yml::Value::String(raw), filled) => {
            if duhem_schema::page_template::has_template_syntax(raw) {
                Ok(())
            } else {
                substitute_with(filled, ctx)
            }
        }
        (serde_yml::Value::Sequence(authored), serde_yml::Value::Sequence(filled)) => {
            for (authored, filled) in authored.iter().zip(filled) {
                resolve_page_entry_nested(authored, filled, ctx)?;
            }
            Ok(())
        }
        (serde_yml::Value::Mapping(authored), serde_yml::Value::Mapping(filled)) => {
            for (key, authored) in authored {
                if let Some(filled) = filled.get_mut(key) {
                    resolve_page_entry_nested(authored, filled, ctx)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
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
    fn page_locator_map_splices_byte_equal_to_inline() {
        let pages: duhem_schema::PageCatalog =
            serde_yml::from_str("login:\n  submit: { role: button, name: Sign In }\n").unwrap();
        let run = run_with(&[]).with_pages(&pages);
        let ctx = RunContext::new(&run);
        let mut authored: serde_yml::Value =
            serde_yml::from_str("locator: $pages.login.submit").unwrap();
        substitute_with(&mut authored, &ctx).expect("catalog resolves");
        let inline: serde_yml::Value =
            serde_yml::from_str("locator: { role: button, name: Sign In }").unwrap();
        assert_eq!(authored, inline);
    }

    #[test]
    fn page_locator_inputs_resolve_after_the_map_splice() {
        let pages: duhem_schema::PageCatalog = serde_yml::from_str(
            "projects:\n  row_delete:\n    role: button\n    name: Delete\n    scope: { role: row, text: $inputs.project_name }\n",
        )
        .unwrap();
        let run = run_with(&[("project_name", Value::Str("Duhem".into()))]).with_pages(&pages);
        let ctx = RunContext::new(&run);
        let mut authored: serde_yml::Value =
            serde_yml::from_str("locator: $pages.projects.row_delete").unwrap();
        substitute_with(&mut authored, &ctx).expect("catalog and nested input resolve");
        let inline: serde_yml::Value = serde_yml::from_str(
            "locator:\n  role: button\n  name: Delete\n  scope: { role: row, text: Duhem }\n",
        )
        .unwrap();
        assert_eq!(authored, inline);
    }

    #[test]
    fn parameterized_page_calls_fill_distinct_selectors() {
        // #495 / #485: one catalog template serves multiple call sites
        // while each action still receives the ordinary locator map.
        let pages: duhem_schema::PageCatalog =
            serde_yml::from_str("chat:\n  history_item:\n    xpath: '(//article)[{}]'\n").unwrap();
        let run = run_with(&[]).with_pages(&pages);
        let ctx = RunContext::new(&run);
        let mut first: serde_yml::Value =
            serde_yml::from_str("locator: $pages.chat.history_item(1)").unwrap();
        let mut second: serde_yml::Value =
            serde_yml::from_str("locator: $pages.chat.history_item(2)").unwrap();
        substitute_with(&mut first, &ctx).expect("first call resolves");
        substitute_with(&mut second, &ctx).expect("second call resolves");

        assert_eq!(first["locator"]["xpath"].as_str(), Some("(//article)[1]"));
        assert_eq!(second["locator"]["xpath"].as_str(), Some("(//article)[2]"));
    }

    #[test]
    fn page_call_resolves_expression_args_and_nested_inputs_without_re_evaluation() {
        let pages: duhem_schema::PageCatalog = serde_yml::from_str(
            "chat:\n  history_item:\n    xpath: '(//article)[{}]'\n    scope: { text: $inputs.scope }\n",
        )
        .unwrap();
        let run = run_with(&[
            ("index", Value::Str("$inputs.secret".into())),
            ("scope", Value::Str("History".into())),
            ("secret", Value::Str("must-not-be-injected".into())),
        ])
        .with_pages(&pages);
        let ctx = RunContext::new(&run);
        let mut authored: serde_yml::Value =
            serde_yml::from_str("locator: $pages.chat.history_item($inputs.index)").unwrap();
        substitute_with(&mut authored, &ctx).expect("page call resolves");

        assert_eq!(
            authored["locator"]["xpath"].as_str(),
            Some("(//article)[$inputs.secret]")
        );
        assert_eq!(
            authored["locator"]["scope"]["text"].as_str(),
            Some("History")
        );
    }

    #[test]
    fn page_call_unescapes_literal_braces() {
        let pages: duhem_schema::PageCatalog =
            serde_yml::from_str("chat:\n  history_item: { css: '{{history}}[{}]' }\n").unwrap();
        let run = run_with(&[]).with_pages(&pages);
        let ctx = RunContext::new(&run);
        let mut authored: serde_yml::Value =
            serde_yml::from_str("locator: $pages.chat.history_item(2)").unwrap();
        substitute_with(&mut authored, &ctx).expect("page call resolves");
        assert_eq!(authored["locator"]["css"].as_str(), Some("{history}[2]"));
    }

    #[test]
    fn unresolved_page_reference_is_a_hard_runtime_error() {
        let run = run_with(&[]);
        let ctx = RunContext::new(&run);
        let mut authored: serde_yml::Value =
            serde_yml::from_str("locator: $pages.login.missing").unwrap();
        let error = substitute_with(&mut authored, &ctx).unwrap_err();
        assert_eq!(error.reference, "$pages.login.missing");
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
    fn malformed_dollar_leading_value_is_an_error() {
        let run = run_with(&[]);
        let ctx = RunContext::new(&run);
        let mut with: serde_yml::Value =
            serde_yml::from_str("{ command: '$inputs.foo bar' }").unwrap();
        let err = substitute_with(&mut with, &ctx).unwrap_err();
        assert_eq!(err.reference, "$inputs.foo bar");
        assert!(
            err.context
                .as_deref()
                .is_some_and(|context| context.starts_with("expression parse error:")),
            "got: {:?}",
            err.context
        );
    }

    #[test]
    fn non_leading_dollar_and_punctuation_pass_through_byte_for_byte() {
        let run = run_with(&[]);
        let ctx = RunContext::new(&run);
        let mut with: serde_yml::Value =
            serde_yml::from_str(r#"{ command: 'echo $inputs.foo {"quoted"}' }"#).unwrap();
        let before = with.clone();
        substitute_with(&mut with, &ctx).expect("plain strings pass through");
        assert_eq!(with, before);
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
