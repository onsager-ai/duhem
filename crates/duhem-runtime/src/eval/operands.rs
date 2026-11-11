//! Observe a condition's actual evaluation, including short-circuiting.

use std::{cell::RefCell, collections::BTreeMap};

use chrono::{DateTime, Utc};
use duhem_schema::{BinOp, Expr, ExprStr, Literal, Path, PathRoot};

use super::{EvalContext, EvalResult, Value};
use crate::engine::context::value_to_json;

pub(crate) fn evaluate(
    expr: &ExprStr,
    inner: &dyn EvalContext,
) -> (EvalResult, BTreeMap<String, serde_json::Value>) {
    let (result, values, _) = evaluate_assertion(&expr.parsed, inner);
    (result, values)
}

/// The last comparison belongs to the root when the root is a comparison:
/// nested comparisons finish first. Logical roots have no comparison detail.
pub(crate) fn evaluate_assertion(
    expr: &Expr,
    inner: &dyn EvalContext,
) -> (
    EvalResult,
    BTreeMap<String, serde_json::Value>,
    Option<String>,
) {
    let ctx = RecordingContext {
        inner,
        values: RefCell::new(BTreeMap::new()),
        comparison: RefCell::new(None),
    };
    let result = super::eval(expr, &ctx);
    let detail = if result == EvalResult::False
        && matches!(expr, Expr::BinOp { op, .. } if !matches!(op, BinOp::And | BinOp::Or))
    {
        ctx.comparison.into_inner()
    } else {
        None
    };
    (result, ctx.values.into_inner(), detail)
}

struct RecordingContext<'a> {
    inner: &'a dyn EvalContext,
    values: RefCell<BTreeMap<String, serde_json::Value>>,
    comparison: RefCell<Option<String>>,
}

impl EvalContext for RecordingContext<'_> {
    fn record_comparison(&self, op: BinOp, lhs: &Value, rhs: &Value) {
        *self.comparison.borrow_mut() = super::comparison_detail(op, lhs, rhs);
    }
    fn input(&self, name: &str) -> Option<&Value> {
        self.inner.input(name)
    }
    fn page(&self, page: &str, element: &str) -> Option<&Value> {
        self.inner.page(page, element)
    }
    fn output(&self, step: &str, output: &str) -> Option<&Value> {
        self.inner.output(step, output)
    }
    fn setup_output(&self, step: &str, output: &str) -> Option<&Value> {
        self.inner.setup_output(step, output)
    }
    fn loop_binding(&self, name: &str) -> Option<&Value> {
        // Without this forward, `EvalContext::loop_binding`'s default
        // no-op impl wins and a `for_each:` (#443) body step's `if:`
        // condition can never resolve `$<as>` once gated through
        // `gating::evaluate` (which always evaluates via this
        // recording wrapper) — every loop-bound value condition would
        // report `MissingLoopBinding` instead of actually gating.
        self.inner.loop_binding(name)
    }
    fn env(&self, name: &str) -> Option<&str> {
        self.inner.env(name)
    }
    fn uuid(&self) -> &str {
        self.inner.uuid()
    }
    fn now(&self) -> DateTime<Utc> {
        self.inner.now()
    }
    fn record_operand(&self, expr: &Expr, value: &Value) {
        if matches!(expr, Expr::Path(_) | Expr::Call { .. }) {
            let mut values = self.values.borrow_mut();
            let label = label(expr);
            // Repeated helpers such as now() may return different values.
            // Retain each evaluation instead of overwriting earlier evidence.
            let mut key = label.clone();
            let mut occurrence = 2;
            while values.contains_key(&key) {
                key = format!("{label} [evaluation {occurrence}]");
                occurrence += 1;
            }
            values.insert(key, value_to_json(value));
        }
    }
}

/// Render a path's `<root>.<segments...>` half, without the leading
/// `$` (callers add that). A `for_each:` loop binding (spec #443)
/// isn't a fixed keyword — `PathRoot::Loop` carries the author's `as:`
/// name as `segments[0]` rather than in the root itself (see
/// `PathRoot::Loop`'s doc comment) — so it renders as `$row.field`,
/// the surface an author actually wrote, not `$loop.row.field`.
fn path_label(path: &Path) -> String {
    let (mut out, start) = match path.root {
        PathRoot::Loop => (path.segments[0].clone(), 1),
        root => (
            if path.segments.is_empty() {
                format!("{}.", root.as_str())
            } else {
                root.as_str().to_string()
            },
            0,
        ),
    };
    for index in start..path.segments.len() {
        let segment = &path.segments[index];
        if !path.quoted_keys.contains(&index) && segment.bytes().all(|b| b.is_ascii_digit()) {
            out.push('.');
            out.push_str(segment);
        } else {
            path.append_segment(&mut out, index);
        }
    }
    out
}

fn label(expr: &Expr) -> String {
    match expr {
        Expr::Path(path) => format!("${}", path_label(path)),
        Expr::Call { path, args } => format!(
            "${}({})",
            path_label(path),
            args.iter().map(label).collect::<Vec<_>>().join(", ")
        ),
        Expr::Lit(lit) => match lit {
            Literal::Bool(v) => v.to_string(),
            Literal::Int(v) => v.to_string(),
            Literal::Float(v) => v.to_string(),
            Literal::Str(v) => serde_json::to_string(v).expect("string serializes"),
        },
        Expr::UnaryOp { expr, .. } => format!("!({})", label(expr)),
        Expr::BinOp { op, lhs, rhs } => {
            let op = match op {
                BinOp::Eq => "==",
                BinOp::Ne => "!=",
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                BinOp::Ge => ">=",
                BinOp::And => "&&",
                BinOp::Or => "||",
            };
            format!("({} {op} {})", label(lhs), label(rhs))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{RunContext, RunState};
    use serde_json::json;

    #[test]
    fn records_nested_operands_but_never_short_circuited_paths() {
        let state = RunState::new(BTreeMap::from([
            (
                "data".into(),
                Value::Object(BTreeMap::from([("count".into(), Value::Int(0))])),
            ),
            ("unused".into(), Value::Int(99)),
        ]));
        let ctx = RunContext::new(&state);
        let expr = ExprStr::from_source("$inputs.data.count > 0 && $inputs.unused > 0").unwrap();
        let (result, operands) = evaluate(&expr, &ctx);
        assert_eq!(result, super::super::eval(&expr.parsed, &ctx));
        assert_eq!(result, EvalResult::False);
        assert_eq!(
            operands,
            BTreeMap::from([("$inputs.data.count".into(), json!(0))])
        );
    }

    #[test]
    fn evidence_renders_quoted_keys_as_parseable_paths() {
        let state = RunState::new(BTreeMap::from([(
            "data".into(),
            Value::Object(BTreeMap::from([("a'b\\c".into(), Value::Int(7))])),
        )]));
        let ctx = RunContext::new(&state);
        let expr = ExprStr::from_source(r"$inputs.data['a\'b\\c'] == 7").unwrap();
        let (result, operands) = evaluate(&expr, &ctx);
        assert_eq!(result, EvalResult::True);
        assert_eq!(operands[r"$inputs.data['a\'b\\c']"], json!(7));
    }

    #[test]
    fn evidence_preserves_dotted_array_indices_and_quoted_numeric_keys() {
        let state = RunState::new(BTreeMap::from([(
            "data".into(),
            Value::Array(vec![Value::Object(BTreeMap::from([
                ("session".into(), Value::Int(7)),
                ("200".into(), Value::Int(8)),
                ("a-b".into(), Value::Int(9)),
            ]))]),
        )]));
        let ctx = RunContext::new(&state);
        for (source, key) in [
            ("$inputs.data[0].session == 7", "$inputs.data.0.session"),
            ("$inputs.data[0]['200'] == 8", "$inputs.data.0['200']"),
            ("$inputs.data[0]['a-b'] == 9", "$inputs.data.0['a-b']"),
        ] {
            let expr = ExprStr::from_source(source).unwrap();
            let (result, operands) = evaluate(&expr, &ctx);
            assert_eq!(result, EvalResult::True);
            assert!(operands.contains_key(key), "{operands:?}");
        }
    }

    #[test]
    fn keeps_each_actual_helper_evaluation() {
        let state = RunState::new(BTreeMap::new());
        let ctx = RunContext::new(&state);
        let expr = ExprStr::from_source("$runtime.uuid() != $runtime.uuid()").unwrap();
        let (result, operands) = evaluate(&expr, &ctx);
        assert_eq!(result, EvalResult::False);
        assert_eq!(operands["$runtime.uuid()"], json!(state.uuid));
        assert_eq!(
            operands["$runtime.uuid() [evaluation 2]"],
            json!(state.uuid)
        );
    }

    #[test]
    fn failure_detail_uses_the_original_helper_evaluation() {
        use std::cell::Cell;
        struct Context {
            calls: Cell<i64>,
        }
        impl EvalContext for Context {
            fn input(&self, _: &str) -> Option<&Value> {
                None
            }
            fn output(&self, _: &str, _: &str) -> Option<&Value> {
                None
            }
            fn env(&self, _: &str) -> Option<&str> {
                None
            }
            fn uuid(&self) -> &str {
                "test"
            }
            fn now(&self) -> DateTime<Utc> {
                let n = self.calls.get();
                self.calls.set(n + 1);
                DateTime::from_timestamp(n, 0).unwrap()
            }
        }
        let ctx = Context {
            calls: Cell::new(0),
        };
        let expr = ExprStr::from_source("$runtime.now() == $runtime.now()").unwrap();
        let (result, values, detail) = evaluate_assertion(&expr.parsed, &ctx);
        assert_eq!(result, EvalResult::False);
        assert_eq!(ctx.calls.get(), 2, "reporting must not invoke now again");
        assert_eq!(values.len(), 2);
        let detail = detail.unwrap();
        assert_eq!(detail, "actual 0, expected 1000");
    }
}
