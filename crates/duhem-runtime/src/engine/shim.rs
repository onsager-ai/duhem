//! `assertion_to_expr` — translate the five closed-enum assertion
//! forms into the same `Expr` shape `eval()` already consumes.
//!
//! Per `docs/duhem-spec.md` §10.6 / the spec on issue #15: the shim
//! is internal — assertion *authoring* shapes are unchanged — and
//! has zero on-the-wire footprint. The wire form
//! (`type_check`, `matches`, `in`, `exists`, `equal`, plus bare
//! boolean) stays exactly what authors write; the shim is the
//! authoring-shape ↔ evaluation-shape bridge.
//!
//! Translation rules:
//!
//! - `Expr(e)` → the parsed `Expr` (no change).
//! - `Equal { values }` → `(values[0] == values[1]) && (values[1] == values[2]) && …`
//! - `In { value, set }` → `(value == set[0]) || (value == set[1]) || …`
//! - `Exists { value }` → `$runtime.exists(value)`
//! - `TypeCheck { value, is }` → `$runtime.type_check(value, "<kind>")`
//! - `Matches { value, pattern }` → `$runtime.matches(value, "<pattern>")`
//!
//! `exists`, `type_check`, and `matches` are runtime built-ins
//! added in this spec to the `$runtime.*` helper catalog (see
//! `eval.rs`). They are *not* part of the schema-layer grammar — the
//! parser already accepts arbitrary `$runtime.<name>(args)` calls.

use duhem_schema::{Assertion, BinOp, Expr, Literal, Path, PathRoot, TypeCheckKind};

/// Translate an `Assertion` into the `Expr` that `eval()` evaluates.
pub fn assertion_to_expr(a: &Assertion) -> Expr {
    match a {
        Assertion::Expr(e) => e.parsed.clone(),

        Assertion::Equal { values } => {
            // Empty / single-element `equal:` is vacuously True
            // (nothing to disagree with). The schema validator's job
            // is to keep authors from writing the degenerate form;
            // the shim stays total.
            if values.len() < 2 {
                return Expr::Lit(Literal::Bool(true));
            }
            let pairs: Vec<Expr> = values
                .windows(2)
                .map(|pair| eq(pair[0].parsed.clone(), pair[1].parsed.clone()))
                .collect();
            chain(BinOp::And, pairs).unwrap_or(Expr::Lit(Literal::Bool(true)))
        }

        Assertion::In { value, set } => {
            // Drop entries that can't be faithfully represented as a
            // v1 scalar literal (Null, mapping, sequence, tagged).
            // They never legitimately match a v1 scalar `value`, so
            // skipping them is equivalent to "no match here"; turning
            // them into a `Bool(false)` literal (the old behavior)
            // would incorrectly match a value of `false`.
            let chunks: Vec<Expr> = set
                .iter()
                .filter_map(|lit| yml_to_expr(lit).map(|e| eq(value.parsed.clone(), e)))
                .collect();
            chain(BinOp::Or, chunks).unwrap_or(Expr::Lit(Literal::Bool(false)))
        }

        Assertion::Exists { value } => Expr::Call {
            path: runtime_path("exists"),
            args: vec![value.parsed.clone()],
        },

        Assertion::TypeCheck { value, is } => Expr::Call {
            path: runtime_path("type_check"),
            args: vec![
                value.parsed.clone(),
                Expr::Lit(Literal::Str(kind_wire(*is).to_string())),
            ],
        },

        Assertion::Matches { value, pattern } => Expr::Call {
            path: runtime_path("matches"),
            args: vec![
                value.parsed.clone(),
                Expr::Lit(Literal::Str(pattern.clone())),
            ],
        },
    }
}

fn eq(lhs: Expr, rhs: Expr) -> Expr {
    Expr::BinOp {
        op: BinOp::Eq,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    }
}

fn chain(op: BinOp, items: Vec<Expr>) -> Option<Expr> {
    let mut iter = items.into_iter();
    let first = iter.next()?;
    Some(iter.fold(first, |acc, next| Expr::BinOp {
        op,
        lhs: Box::new(acc),
        rhs: Box::new(next),
    }))
}

fn runtime_path(name: &str) -> Path {
    Path {
        root: PathRoot::Runtime,
        segments: vec![name.to_string()],
    }
}

fn kind_wire(k: TypeCheckKind) -> &'static str {
    match k {
        TypeCheckKind::Uuid => "uuid",
        TypeCheckKind::String => "string",
        TypeCheckKind::Integer => "integer",
        TypeCheckKind::Float => "float",
        TypeCheckKind::Boolean => "boolean",
        TypeCheckKind::Object => "object",
        TypeCheckKind::Array => "array",
        TypeCheckKind::Null => "null",
    }
}

/// Convert a YAML literal in an `in:` set into the equivalent `Expr`
/// literal. Returns `None` for entries the v1 scalar value model
/// cannot faithfully represent — Null (no `Lit::Null` in the
/// expression AST) and composite shapes (mapping/sequence/tagged).
/// `assertion_to_expr` filters those out so they neither match nor
/// false-match: an entry of `{}` would otherwise collapse to `false`
/// and *would* match a checked value of `false`. The right answer is
/// "we can't represent this; skip"; the wrong answer is "pretend it's
/// some scalar that might collide".
fn yml_to_expr(v: &serde_yml::Value) -> Option<Expr> {
    use serde_yml::Value;
    Some(match v {
        Value::Bool(b) => Expr::Lit(Literal::Bool(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Expr::Lit(Literal::Int(i))
            } else if let Some(f) = n.as_f64() {
                Expr::Lit(Literal::Float(f))
            } else {
                return None;
            }
        }
        Value::String(s) => Expr::Lit(Literal::Str(s.clone())),
        Value::Null | Value::Sequence(_) | Value::Mapping(_) | Value::Tagged(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{EvalContext, EvalResult, InconclusiveCause, Value as RtValue, eval};
    use chrono::{DateTime, Utc};
    use duhem_schema::{ExprStr, TypeCheckKind};
    use std::collections::HashMap;

    struct Ctx {
        inputs: HashMap<String, RtValue>,
        outputs: HashMap<(String, String), RtValue>,
    }

    impl Ctx {
        fn new() -> Self {
            Self {
                inputs: HashMap::new(),
                outputs: HashMap::new(),
            }
        }
        fn with_input(mut self, k: &str, v: RtValue) -> Self {
            self.inputs.insert(k.into(), v);
            self
        }
        fn with_output(mut self, step: &str, out: &str, v: RtValue) -> Self {
            self.outputs.insert((step.into(), out.into()), v);
            self
        }
    }

    impl EvalContext for Ctx {
        fn input(&self, name: &str) -> Option<&RtValue> {
            self.inputs.get(name)
        }
        fn output(&self, step: &str, output: &str) -> Option<&RtValue> {
            self.outputs.get(&(step.into(), output.into()))
        }
        fn env(&self, _: &str) -> Option<&str> {
            None
        }
        fn uuid(&self) -> &str {
            "ctx"
        }
        fn now(&self) -> DateTime<Utc> {
            Utc::now()
        }
    }

    fn es(src: &str) -> ExprStr {
        ExprStr::from_source(src).unwrap()
    }

    #[test]
    fn bare_expression_passes_through() {
        let a = Assertion::Expr(es("1 == 1"));
        let e = assertion_to_expr(&a);
        assert_eq!(eval(&e, &Ctx::new()), EvalResult::True);
    }

    #[test]
    fn equal_chains_eq_pairs() {
        let v = vec![es("1"), es("1"), es("1")];
        let a = Assertion::Equal { values: v };
        assert_eq!(eval(&assertion_to_expr(&a), &Ctx::new()), EvalResult::True);

        let v = vec![es("1"), es("2")];
        let a = Assertion::Equal { values: v };
        assert_eq!(eval(&assertion_to_expr(&a), &Ctx::new()), EvalResult::False);
    }

    #[test]
    fn equal_propagates_inconclusive_from_missing_step() {
        let v = vec![es("$steps.missing.outputs.x"), es("1")];
        let a = Assertion::Equal { values: v };
        let r = eval(&assertion_to_expr(&a), &Ctx::new());
        assert!(matches!(
            r,
            EvalResult::Inconclusive(InconclusiveCause::MissingObservation { .. })
        ));
    }

    #[test]
    fn in_set_maps_to_or_chain() {
        let a = Assertion::In {
            value: es("$inputs.x"),
            set: vec![
                serde_yml::Value::from(1),
                serde_yml::Value::from(2),
                serde_yml::Value::from(3),
            ],
        };
        let ctx = Ctx::new().with_input("x", RtValue::Int(2));
        assert_eq!(eval(&assertion_to_expr(&a), &ctx), EvalResult::True);

        let ctx = Ctx::new().with_input("x", RtValue::Int(99));
        assert_eq!(eval(&assertion_to_expr(&a), &ctx), EvalResult::False);
    }

    #[test]
    fn in_set_skips_non_scalar_entries_without_false_matching_a_bool() {
        // Regression for the lossy `yml_to_expr → Bool(false)` mapping.
        // A set entry of `{}` (empty mapping) must not match the
        // checked value `false`.
        let empty_mapping = serde_yml::from_str::<serde_yml::Value>("{}").unwrap();
        let a = Assertion::In {
            value: es("$inputs.x"),
            set: vec![empty_mapping],
        };
        let ctx = Ctx::new().with_input("x", RtValue::Bool(false));
        assert_eq!(eval(&assertion_to_expr(&a), &ctx), EvalResult::False);
    }

    #[test]
    fn exists_true_when_observed_false_when_not() {
        let a = Assertion::Exists {
            value: es("$steps.s.outputs.y"),
        };
        let observed = Ctx::new().with_output("s", "y", RtValue::Int(0));
        let missing = Ctx::new();
        assert_eq!(eval(&assertion_to_expr(&a), &observed), EvalResult::True);
        assert_eq!(eval(&assertion_to_expr(&a), &missing), EvalResult::False);
    }

    #[test]
    fn type_check_matches_known_kinds() {
        let a = Assertion::TypeCheck {
            value: es("$inputs.x"),
            is: TypeCheckKind::String,
        };
        let ctx = Ctx::new().with_input("x", RtValue::Str("hi".into()));
        assert_eq!(eval(&assertion_to_expr(&a), &ctx), EvalResult::True);

        let a = Assertion::TypeCheck {
            value: es("$inputs.x"),
            is: TypeCheckKind::Integer,
        };
        assert_eq!(eval(&assertion_to_expr(&a), &ctx), EvalResult::False);

        let a = Assertion::TypeCheck {
            value: es("$inputs.x"),
            is: TypeCheckKind::Uuid,
        };
        let valid = Ctx::new().with_input(
            "x",
            RtValue::Str("018f5e8a-7c3a-7a8e-9a4b-1c0d2e3f4a5b".into()),
        );
        assert_eq!(eval(&assertion_to_expr(&a), &valid), EvalResult::True);
        let bad = Ctx::new().with_input("x", RtValue::Str("not-a-uuid".into()));
        assert_eq!(eval(&assertion_to_expr(&a), &bad), EvalResult::False);
    }

    #[test]
    fn matches_regex() {
        let a = Assertion::Matches {
            value: es("$inputs.x"),
            pattern: "^foo[0-9]+$".into(),
        };
        let ok = Ctx::new().with_input("x", RtValue::Str("foo42".into()));
        let bad = Ctx::new().with_input("x", RtValue::Str("bar".into()));
        assert_eq!(eval(&assertion_to_expr(&a), &ok), EvalResult::True);
        assert_eq!(eval(&assertion_to_expr(&a), &bad), EvalResult::False);
    }
}
