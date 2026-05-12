//! Three-state expression evaluator over the v0.1 schema AST.
//!
//! Per `docs/duhem-spec.md` §10.6 / §10.7: turns a parsed
//! `duhem_schema::Expr` into a `True | False | Inconclusive` verdict
//! against a runtime context (declared inputs, observed step outputs,
//! whitelisted env, runtime helpers). Mechanical judgment only — no
//! LLM in the loop. Inconclusive is the load-bearing third state: it
//! lets the judge distinguish *fail* (the system did the wrong thing)
//! from *we couldn't tell*.

use chrono::{DateTime, Utc};
use duhem_schema::{BinOp, Expr, Literal, Path, PathRoot, UnaryOp};

/// Verdict for a single boolean expression. Matches the judge's
/// three-valued verdict shape (`pass | fail | inconclusive`).
#[derive(Debug, Clone, PartialEq)]
pub enum EvalResult {
    True,
    False,
    Inconclusive(InconclusiveCause),
}

/// Why an evaluation could not produce a definitive verdict. Closed
/// set at v1; new variants are an evaluator-level change, not a
/// schema-level one.
#[derive(Debug, Clone, PartialEq)]
pub enum InconclusiveCause {
    /// `$steps.X.outputs.Y` references a step output that was never
    /// produced (timed out, didn't run, extractor returned no value).
    MissingObservation { step: String, output: String },
    /// `$inputs.X` not present at run time. Defense-in-depth — the
    /// schema validator should catch this at authoring time.
    MissingInput(String),
    /// `$env.X` not in the whitelisted env at run time.
    MissingEnv(String),
    /// `$runtime.fn(...)` for a `fn` outside the closed v1 helper set.
    UnknownRuntimeHelper(String),
    /// Comparison applied to non-comparable shapes, e.g. `"a" < 5`.
    TypeMismatch { lhs: ValueShape, rhs: ValueShape },
    /// A `$runtime.matches(value, pattern)` pattern that did not
    /// compile as a regex. The operands are well-typed; the regex
    /// source itself is malformed. Carries the regex-engine error
    /// message so evidence `detail` can guide the author back to the
    /// offending pattern.
    InvalidPattern(String),
}

/// Runtime-side value. Distinct from `duhem_schema::Literal` because
/// runtime values come from observed JSON, declared inputs, or
/// extracted strings — not authored literals.
///
/// Scalars (`Bool` / `Int` / `Float` / `Str` / `Null`) participate in
/// the full comparison surface. The `Array` / `Object` variants exist
/// so the typed-input catalog can carry declared `array` / `object`
/// inputs end-to-end (`type_check: { value: $inputs.x, is: object }`);
/// they are not comparable at v1 — operations against them produce
/// `Inconclusive(TypeMismatch)`. Equality / ordering for collections
/// is a separate spec; the grammar has no array/object *literals*
/// today, so the only authored interactions are `type_check` and
/// (via #15's template substitution) splicing into `Step.with`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Null,
    Array(Vec<Value>),
    Object(std::collections::BTreeMap<String, Value>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueShape {
    Bool,
    Int,
    Float,
    Str,
    Null,
    Array,
    Object,
}

impl Value {
    pub fn shape(&self) -> ValueShape {
        match self {
            Value::Bool(_) => ValueShape::Bool,
            Value::Int(_) => ValueShape::Int,
            Value::Float(_) => ValueShape::Float,
            Value::Str(_) => ValueShape::Str,
            Value::Null => ValueShape::Null,
            Value::Array(_) => ValueShape::Array,
            Value::Object(_) => ValueShape::Object,
        }
    }
}

/// Runtime-side bindings for an in-flight check. The step executor
/// implements this; the evaluator never reaches past it.
pub trait EvalContext {
    fn input(&self, name: &str) -> Option<&Value>;
    fn output(&self, step_id: &str, output: &str) -> Option<&Value>;
    fn env(&self, name: &str) -> Option<&str>;
    /// UUID for this run; cached on the context so a definition that
    /// uses `$runtime.uuid()` twice gets the same value (the
    /// `"test-ws-{{uuid}}"` author-intent pattern from
    /// `docs/duhem-spec.md` §10.3).
    fn uuid(&self) -> &str;
    /// Wall-clock at the call site. Sampled fresh on each call.
    fn now(&self) -> DateTime<Utc>;
}

/// Evaluate a parsed expression against a runtime context.
pub fn eval(expr: &Expr, ctx: &dyn EvalContext) -> EvalResult {
    match eval_value(expr, ctx) {
        Ok(Value::Bool(true)) => EvalResult::True,
        Ok(Value::Bool(false)) => EvalResult::False,
        Ok(v) => EvalResult::Inconclusive(InconclusiveCause::TypeMismatch {
            lhs: v.shape(),
            rhs: ValueShape::Bool,
        }),
        Err(cause) => EvalResult::Inconclusive(cause),
    }
}

/// Evaluate an expression to its raw scalar `Value`. Crate-internal
/// wrapper over the value evaluator so the engine's `Step.with`
/// template substitution can resolve `$inputs.X` / `$runtime.X()`
/// references without going through the boolean-only `eval()`. Kept
/// `pub(crate)` because it's an engine-internal helper — not a
/// supported public API.
pub(crate) fn eval_to_value(
    expr: &Expr,
    ctx: &dyn EvalContext,
) -> Result<Value, InconclusiveCause> {
    eval_value(expr, ctx)
}

type EvalRes = Result<Value, InconclusiveCause>;

fn eval_value(expr: &Expr, ctx: &dyn EvalContext) -> EvalRes {
    match expr {
        Expr::Lit(l) => Ok(literal_to_value(l)),
        Expr::Path(p) => eval_path(p, ctx),
        Expr::Call { path, args } => eval_call(path, args, ctx),
        Expr::BinOp { op, lhs, rhs } => match op {
            BinOp::And | BinOp::Or => eval_logical(*op, lhs, rhs, ctx),
            _ => eval_compare(*op, lhs, rhs, ctx),
        },
        Expr::UnaryOp { op, expr } => match op {
            UnaryOp::Not => match eval_value(expr, ctx)? {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                v => Err(InconclusiveCause::TypeMismatch {
                    lhs: v.shape(),
                    rhs: ValueShape::Bool,
                }),
            },
        },
    }
}

fn literal_to_value(l: &Literal) -> Value {
    match l {
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Int(i) => Value::Int(*i),
        Literal::Float(f) => Value::Float(*f),
        Literal::Str(s) => Value::Str(s.clone()),
    }
}

fn eval_path(p: &Path, ctx: &dyn EvalContext) -> EvalRes {
    match p.root {
        PathRoot::Inputs => {
            // Schema validator guarantees a single segment for
            // well-formed `$inputs.<name>` references.
            let name = p.segments.first().map(String::as_str).unwrap_or("");
            ctx.input(name)
                .cloned()
                .ok_or_else(|| InconclusiveCause::MissingInput(name.to_string()))
        }
        PathRoot::Steps => {
            // Schema validator guarantees the
            // `$steps.<step_id>.outputs.<output>` shape.
            let step = p.segments.first().map(String::as_str).unwrap_or("");
            let output = p.segments.get(2).map(String::as_str).unwrap_or("");
            ctx.output(step, output)
                .cloned()
                .ok_or_else(|| InconclusiveCause::MissingObservation {
                    step: step.to_string(),
                    output: output.to_string(),
                })
        }
        PathRoot::Env => {
            let name = p.segments.first().map(String::as_str).unwrap_or("");
            ctx.env(name)
                .map(|s| Value::Str(s.to_string()))
                .ok_or_else(|| InconclusiveCause::MissingEnv(name.to_string()))
        }
        PathRoot::Runtime => {
            // Bare `$runtime.<name>` (no call): helpers must be called.
            let name = p.segments.join(".");
            Err(InconclusiveCause::UnknownRuntimeHelper(name))
        }
    }
}

fn eval_call(path: &Path, args: &[Expr], ctx: &dyn EvalContext) -> EvalRes {
    // The schema parser only allows `(...)` under `$runtime`, so
    // `path.root` is `Runtime` here for any well-parsed input.
    let helper = match path.segments.as_slice() {
        [name] => name.as_str(),
        _ => {
            return Err(InconclusiveCause::UnknownRuntimeHelper(
                path.segments.join("."),
            ));
        }
    };
    match (helper, args.len()) {
        ("uuid", 0) => Ok(Value::Str(ctx.uuid().to_string())),
        ("now", 0) => Ok(Value::Int(ctx.now().timestamp_millis())),
        // `exists(value)`: True if the value path resolves to a
        // present scalar, False if any underlying lookup reports
        // missing. Anything else (e.g. TypeMismatch) propagates as
        // Inconclusive. The closed-enum `Assertion::Exists` shim in
        // the engine emits this call.
        ("exists", 1) => match eval_value(&args[0], ctx) {
            Ok(_) => Ok(Value::Bool(true)),
            Err(
                InconclusiveCause::MissingObservation { .. }
                | InconclusiveCause::MissingInput(_)
                | InconclusiveCause::MissingEnv(_),
            ) => Ok(Value::Bool(false)),
            Err(c) => Err(c),
        },
        // `matches(value, pattern)`: regex match against a string.
        // Both args must evaluate to strings; non-string operands
        // surface as Inconclusive(TypeMismatch) the same way the
        // comparison operators do.
        ("matches", 2) => {
            let v = eval_value(&args[0], ctx)?;
            let p = eval_value(&args[1], ctx)?;
            let (s, pat) = match (&v, &p) {
                (Value::Str(s), Value::Str(p)) => (s.clone(), p.clone()),
                _ => {
                    return Err(InconclusiveCause::TypeMismatch {
                        lhs: v.shape(),
                        rhs: p.shape(),
                    });
                }
            };
            let re = regex::Regex::new(&pat)
                .map_err(|e| InconclusiveCause::InvalidPattern(e.to_string()))?;
            Ok(Value::Bool(re.is_match(&s)))
        }
        // `type_check(value, kind)`: structural shape check. `kind`
        // is a string literal carrying the snake_case wire form of
        // `TypeCheckKind` (`string`, `integer`, …, `uuid`). The
        // `object` / `array` kinds aren't representable in the v1
        // scalar value model and always evaluate to False — there is
        // no scalar value that *is* an object.
        ("type_check", 2) => {
            let v = eval_value(&args[0], ctx)?;
            let kind = match eval_value(&args[1], ctx)? {
                Value::Str(s) => s,
                other => {
                    return Err(InconclusiveCause::TypeMismatch {
                        lhs: other.shape(),
                        rhs: ValueShape::Str,
                    });
                }
            };
            Ok(Value::Bool(matches_type(&v, &kind)))
        }
        _ => Err(InconclusiveCause::UnknownRuntimeHelper(helper.to_string())),
    }
}

fn matches_type(v: &Value, kind: &str) -> bool {
    match (v, kind) {
        (Value::Str(s), "uuid") => uuid::Uuid::parse_str(s).is_ok(),
        (Value::Str(_), "string") => true,
        (Value::Int(_), "integer") => true,
        // `number` matches any numeric shape; `integer` is a subset.
        (Value::Int(_) | Value::Float(_), "number") => true,
        (Value::Float(_), "float") => true,
        (Value::Bool(_), "boolean") => true,
        (Value::Null, "null") => true,
        (Value::Array(_), "array") => true,
        (Value::Object(_), "object") => true,
        _ => false,
    }
}

/// Kleene three-valued logic with true short-circuit on the left
/// operand: `true || X` and `false && X` skip evaluating `X` entirely
/// (so missing-context lookups, helper calls, and any future
/// observable effects don't fire on the bypassed side). When `lhs` is
/// inconclusive we still have to evaluate `rhs` because
/// `Inconclusive || true` is `True` and `Inconclusive && false` is
/// `False` under Kleene semantics.
fn eval_logical(op: BinOp, lhs: &Expr, rhs: &Expr, ctx: &dyn EvalContext) -> EvalRes {
    use Bool3::*;
    let l = to_bool3(eval_value(lhs, ctx));
    match (op, &l) {
        (BinOp::Or, T) => return Ok(Value::Bool(true)),
        (BinOp::And, F) => return Ok(Value::Bool(false)),
        _ => {}
    }
    let r = to_bool3(eval_value(rhs, ctx));
    let out = match (op, l, r) {
        (BinOp::Or, _, T) => T,
        (BinOp::Or, F, F) => F,
        (BinOp::And, _, F) => F,
        (BinOp::And, T, T) => T,
        // Anything else reduces to inconclusive — pick the left cause
        // when present so the failure surface is deterministic.
        (_, I(c), _) | (_, _, I(c)) => I(c),
        _ => unreachable!("logical op with non-logical or unhandled operand pair"),
    };
    match out {
        T => Ok(Value::Bool(true)),
        F => Ok(Value::Bool(false)),
        I(c) => Err(c),
    }
}

enum Bool3 {
    T,
    F,
    I(InconclusiveCause),
}

fn to_bool3(r: EvalRes) -> Bool3 {
    match r {
        Ok(Value::Bool(true)) => Bool3::T,
        Ok(Value::Bool(false)) => Bool3::F,
        Ok(v) => Bool3::I(InconclusiveCause::TypeMismatch {
            lhs: v.shape(),
            rhs: ValueShape::Bool,
        }),
        Err(c) => Bool3::I(c),
    }
}

fn eval_compare(op: BinOp, lhs: &Expr, rhs: &Expr, ctx: &dyn EvalContext) -> EvalRes {
    let l = eval_value(lhs, ctx)?;
    let r = eval_value(rhs, ctx)?;
    compare(op, &l, &r)
}

fn compare(op: BinOp, l: &Value, r: &Value) -> EvalRes {
    use Value::*;
    let res: Option<bool> = match (l, r) {
        (Bool(a), Bool(b)) => match op {
            BinOp::Eq => Some(a == b),
            BinOp::Ne => Some(a != b),
            // Booleans aren't ordered at v1.
            _ => None,
        },
        (Str(a), Str(b)) => Some(apply_ord(op, a, b)),
        (Int(a), Int(b)) => Some(apply_ord(op, a, b)),
        (Float(a), Float(b)) => Some(apply_ord(op, a, b)),
        (Int(a), Float(b)) => Some(apply_ord(op, &(*a as f64), b)),
        (Float(a), Int(b)) => Some(apply_ord(op, a, &(*b as f64))),
        (Null, Null) => match op {
            BinOp::Eq => Some(true),
            BinOp::Ne => Some(false),
            _ => None,
        },
        _ => None,
    };
    match res {
        Some(b) => Ok(Value::Bool(b)),
        None => Err(InconclusiveCause::TypeMismatch {
            lhs: l.shape(),
            rhs: r.shape(),
        }),
    }
}

fn apply_ord<T: PartialOrd>(op: BinOp, a: &T, b: &T) -> bool {
    match op {
        BinOp::Eq => a == b,
        BinOp::Ne => a != b,
        BinOp::Lt => a < b,
        BinOp::Le => a <= b,
        BinOp::Gt => a > b,
        BinOp::Ge => a >= b,
        _ => unreachable!("apply_ord called with non-comparison op"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::HashMap;

    use duhem_schema::expr::parse;

    struct TestCtx {
        inputs: HashMap<String, Value>,
        outputs: HashMap<(String, String), Value>,
        env: HashMap<String, String>,
        uuid: String,
        now: DateTime<Utc>,
    }

    impl TestCtx {
        fn new() -> Self {
            Self {
                inputs: HashMap::new(),
                outputs: HashMap::new(),
                env: HashMap::new(),
                uuid: "ctx-uuid".to_string(),
                now: Utc::now(),
            }
        }
        fn with_input(mut self, k: &str, v: Value) -> Self {
            self.inputs.insert(k.to_string(), v);
            self
        }
        fn with_output(mut self, step: &str, out: &str, v: Value) -> Self {
            self.outputs.insert((step.to_string(), out.to_string()), v);
            self
        }
        fn with_env(mut self, k: &str, v: &str) -> Self {
            self.env.insert(k.to_string(), v.to_string());
            self
        }
        fn with_uuid(mut self, u: &str) -> Self {
            self.uuid = u.to_string();
            self
        }
    }

    impl EvalContext for TestCtx {
        fn input(&self, name: &str) -> Option<&Value> {
            self.inputs.get(name)
        }
        fn output(&self, step_id: &str, output: &str) -> Option<&Value> {
            self.outputs.get(&(step_id.to_string(), output.to_string()))
        }
        fn env(&self, name: &str) -> Option<&str> {
            self.env.get(name).map(String::as_str)
        }
        fn uuid(&self) -> &str {
            &self.uuid
        }
        fn now(&self) -> DateTime<Utc> {
            self.now
        }
    }

    fn run(src: &str, ctx: &dyn EvalContext) -> EvalResult {
        let e = parse(src).expect("parse");
        eval(&e, ctx)
    }

    // ---- truth table over literals --------------------------------

    #[test]
    fn literal_truth_table() {
        let ctx = TestCtx::new();
        // boolean literals
        assert_eq!(run("true", &ctx), EvalResult::True);
        assert_eq!(run("false", &ctx), EvalResult::False);
        assert_eq!(run("!true", &ctx), EvalResult::False);
        assert_eq!(run("!false", &ctx), EvalResult::True);
        // Eq / Ne across each scalar shape
        assert_eq!(run("1 == 1", &ctx), EvalResult::True);
        assert_eq!(run("1 == 2", &ctx), EvalResult::False);
        assert_eq!(run("1 != 2", &ctx), EvalResult::True);
        assert_eq!(run("\"a\" == \"a\"", &ctx), EvalResult::True);
        assert_eq!(run("\"a\" == \"b\"", &ctx), EvalResult::False);
        assert_eq!(run("true == true", &ctx), EvalResult::True);
        assert_eq!(run("true != false", &ctx), EvalResult::True);
        // ordering
        assert_eq!(run("1 < 2", &ctx), EvalResult::True);
        assert_eq!(run("2 <= 2", &ctx), EvalResult::True);
        assert_eq!(run("3 > 2", &ctx), EvalResult::True);
        assert_eq!(run("3 >= 4", &ctx), EvalResult::False);
        assert_eq!(run("\"a\" < \"b\"", &ctx), EvalResult::True);
        // numeric promotion
        assert_eq!(run("1 == 1.0", &ctx), EvalResult::True);
        assert_eq!(run("1 < 1.5", &ctx), EvalResult::True);
        // boolean composition
        assert_eq!(run("true && false", &ctx), EvalResult::False);
        assert_eq!(run("true && true", &ctx), EvalResult::True);
        assert_eq!(run("false || true", &ctx), EvalResult::True);
        assert_eq!(run("false || false", &ctx), EvalResult::False);
    }

    // ---- inconclusive causes --------------------------------------

    #[test]
    fn missing_step_output_is_inconclusive() {
        let ctx = TestCtx::new();
        assert_eq!(
            run("$steps.missing.outputs.x == 200", &ctx),
            EvalResult::Inconclusive(InconclusiveCause::MissingObservation {
                step: "missing".into(),
                output: "x".into(),
            })
        );
    }

    #[test]
    fn missing_input_is_inconclusive() {
        let ctx = TestCtx::new();
        assert_eq!(
            run("$inputs.nope == 1", &ctx),
            EvalResult::Inconclusive(InconclusiveCause::MissingInput("nope".into()))
        );
    }

    #[test]
    fn missing_env_is_inconclusive() {
        let ctx = TestCtx::new();
        assert_eq!(
            run("$env.NOT_SET == \"x\"", &ctx),
            EvalResult::Inconclusive(InconclusiveCause::MissingEnv("NOT_SET".into()))
        );
    }

    #[test]
    fn type_mismatch_is_inconclusive() {
        let ctx = TestCtx::new();
        assert_eq!(
            run("\"foo\" < 5", &ctx),
            EvalResult::Inconclusive(InconclusiveCause::TypeMismatch {
                lhs: ValueShape::Str,
                rhs: ValueShape::Int,
            })
        );
    }

    #[test]
    fn ordering_on_bool_is_type_mismatch() {
        let ctx = TestCtx::new();
        assert_eq!(
            run("true < false", &ctx),
            EvalResult::Inconclusive(InconclusiveCause::TypeMismatch {
                lhs: ValueShape::Bool,
                rhs: ValueShape::Bool,
            })
        );
    }

    #[test]
    fn non_bool_top_level_is_type_mismatch() {
        let ctx = TestCtx::new().with_input("name", Value::Str("ws".into()));
        assert_eq!(
            run("$inputs.name", &ctx),
            EvalResult::Inconclusive(InconclusiveCause::TypeMismatch {
                lhs: ValueShape::Str,
                rhs: ValueShape::Bool,
            })
        );
    }

    // ---- runtime helpers -----------------------------------------

    #[test]
    fn uuid_is_stable_within_a_context() {
        let ctx = TestCtx::new().with_uuid("run-42");
        let e = parse("$runtime.uuid() == $runtime.uuid()").unwrap();
        assert_eq!(eval(&e, &ctx), EvalResult::True);
    }

    #[test]
    fn uuid_differs_across_contexts() {
        let a = TestCtx::new().with_uuid("a");
        let b = TestCtx::new().with_uuid("b");
        let e = parse("$runtime.uuid()").unwrap();
        // Each context yields its own value; pull both via comparison
        // against a literal so we can assert without exposing Value at
        // the top of `eval`.
        let ea = parse("$runtime.uuid() == \"a\"").unwrap();
        let eb = parse("$runtime.uuid() == \"a\"").unwrap();
        assert_eq!(eval(&ea, &a), EvalResult::True);
        assert_eq!(eval(&eb, &b), EvalResult::False);
        // And the bare-call form is a (non-bool) string under each.
        match eval(&e, &a) {
            EvalResult::Inconclusive(InconclusiveCause::TypeMismatch { lhs, .. }) => {
                assert_eq!(lhs, ValueShape::Str);
            }
            other => panic!("expected TypeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn unknown_runtime_helper_is_inconclusive() {
        let ctx = TestCtx::new();
        assert_eq!(
            run("$runtime.bogus()", &ctx),
            EvalResult::Inconclusive(InconclusiveCause::UnknownRuntimeHelper("bogus".into()))
        );
    }

    #[test]
    fn bare_runtime_path_is_unknown_helper() {
        // `$runtime.uuid` with no call form — helpers must be called.
        let ctx = TestCtx::new();
        assert_eq!(
            run("$runtime.uuid", &ctx),
            EvalResult::Inconclusive(InconclusiveCause::UnknownRuntimeHelper("uuid".into()))
        );
    }

    #[test]
    fn now_returns_int_millis() {
        let ctx = TestCtx::new();
        // `now() == now()` within a single eval samples the same
        // mocked clock, so the comparison is True under TestCtx.
        let e = parse("$runtime.now() == $runtime.now()").unwrap();
        assert_eq!(eval(&e, &ctx), EvalResult::True);
    }

    /// Context whose `now()` advances by one millisecond every call.
    /// Used to verify the evaluator does not cache `now()` across call
    /// sites within a single `eval`.
    struct TickingCtx {
        base: DateTime<Utc>,
        counter: Cell<i64>,
    }

    impl EvalContext for TickingCtx {
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
            "ticking"
        }
        fn now(&self) -> DateTime<Utc> {
            let n = self.counter.get();
            self.counter.set(n + 1);
            self.base + chrono::Duration::milliseconds(n)
        }
    }

    #[test]
    fn now_is_sampled_fresh_on_each_call_site() {
        let ctx = TickingCtx {
            base: DateTime::<Utc>::from_timestamp_millis(1_700_000_000_000).unwrap(),
            counter: Cell::new(0),
        };
        // First call yields base+0ms, second yields base+1ms, so the
        // first sample is strictly less than the second. If the
        // evaluator cached `now()`, this would be False or
        // Inconclusive instead.
        let e = parse("$runtime.now() < $runtime.now()").unwrap();
        assert_eq!(eval(&e, &ctx), EvalResult::True);
    }

    // ---- short-circuit in the presence of inconclusive ----------

    #[test]
    fn or_short_circuits_past_inconclusive() {
        let ctx = TestCtx::new();
        assert_eq!(
            run("true || $steps.missing.outputs.x == 1", &ctx),
            EvalResult::True
        );
        // symmetric: inconclusive on the left, true on the right
        assert_eq!(
            run("$steps.missing.outputs.x == 1 || true", &ctx),
            EvalResult::True
        );
    }

    #[test]
    fn and_short_circuits_past_inconclusive() {
        let ctx = TestCtx::new();
        assert_eq!(
            run("false && $steps.missing.outputs.x == 1", &ctx),
            EvalResult::False
        );
        assert_eq!(
            run("$steps.missing.outputs.x == 1 && false", &ctx),
            EvalResult::False
        );
    }

    /// Context that counts every `output()` lookup so a test can
    /// assert the right-hand side of a short-circuited boolean was
    /// never evaluated.
    struct CountingCtx {
        outputs: HashMap<(String, String), Value>,
        lookups: Cell<usize>,
    }

    impl EvalContext for CountingCtx {
        fn input(&self, _: &str) -> Option<&Value> {
            None
        }
        fn output(&self, step_id: &str, output: &str) -> Option<&Value> {
            self.lookups.set(self.lookups.get() + 1);
            self.outputs.get(&(step_id.to_string(), output.to_string()))
        }
        fn env(&self, _: &str) -> Option<&str> {
            None
        }
        fn uuid(&self) -> &str {
            "counting"
        }
        fn now(&self) -> DateTime<Utc> {
            Utc::now()
        }
    }

    #[test]
    fn or_does_not_evaluate_rhs_when_lhs_is_true() {
        let ctx = CountingCtx {
            outputs: HashMap::new(),
            lookups: Cell::new(0),
        };
        // `true` short-circuits; `$steps.x.outputs.y` must not be
        // looked up at all.
        let e = parse("true || $steps.x.outputs.y == 1").unwrap();
        assert_eq!(eval(&e, &ctx), EvalResult::True);
        assert_eq!(ctx.lookups.get(), 0);
    }

    #[test]
    fn and_does_not_evaluate_rhs_when_lhs_is_false() {
        let ctx = CountingCtx {
            outputs: HashMap::new(),
            lookups: Cell::new(0),
        };
        let e = parse("false && $steps.x.outputs.y == 1").unwrap();
        assert_eq!(eval(&e, &ctx), EvalResult::False);
        assert_eq!(ctx.lookups.get(), 0);
    }

    #[test]
    fn or_propagates_inconclusive_when_other_side_is_false() {
        let ctx = TestCtx::new();
        let r = run("false || $steps.missing.outputs.x == 1", &ctx);
        assert!(matches!(
            r,
            EvalResult::Inconclusive(InconclusiveCause::MissingObservation { .. })
        ));
    }

    #[test]
    fn and_propagates_inconclusive_when_other_side_is_true() {
        let ctx = TestCtx::new();
        let r = run("true && $steps.missing.outputs.x == 1", &ctx);
        assert!(matches!(
            r,
            EvalResult::Inconclusive(InconclusiveCause::MissingObservation { .. })
        ));
    }

    // ---- worked example from the spec ---------------------------

    #[test]
    fn worked_example_status_eq_200() {
        let ok = TestCtx::new().with_output("api_call", "status", Value::Int(200));
        let bad = TestCtx::new().with_output("api_call", "status", Value::Int(500));
        let none = TestCtx::new();
        assert_eq!(
            run("$steps.api_call.outputs.status == 200", &ok),
            EvalResult::True
        );
        assert_eq!(
            run("$steps.api_call.outputs.status == 200", &bad),
            EvalResult::False
        );
        assert_eq!(
            run("$steps.api_call.outputs.status == 200", &none),
            EvalResult::Inconclusive(InconclusiveCause::MissingObservation {
                step: "api_call".into(),
                output: "status".into(),
            })
        );
    }

    #[test]
    fn env_value_compares_as_string() {
        let ctx = TestCtx::new().with_env("DATABASE_URL", "postgres://x");
        assert_eq!(
            run("$env.DATABASE_URL == \"postgres://x\"", &ctx),
            EvalResult::True
        );
    }
}
