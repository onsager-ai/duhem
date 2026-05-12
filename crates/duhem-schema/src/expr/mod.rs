//! Expression AST for Verification-Definition assertions.
//!
//! v0.1 grammar — boolean expressions over scoped path references and
//! literals, plus simple `$runtime.fn(args)` calls. Parsing happens at
//! schema-load time so `cargo run -- validate` surfaces syntax errors
//! before the runtime is ever invoked. Evaluation is the runtime's job
//! (`spec(runtime): expression evaluator v1`).
//!
//! Scope of v0.1: comparison + boolean logic + paths + literals. No
//! arithmetic, no string concatenation, no array indexing — those land
//! when a Verification Definition needs them.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod parse;

pub use parse::{ParseError, parse};

/// A parsed expression, with the original authored source preserved
/// alongside it. `raw` is what we re-emit on YAML round-trip;
/// `parsed` is what the runtime evaluator and validator inspect.
#[derive(Debug, Clone)]
pub struct ExprStr {
    pub raw: String,
    pub parsed: Expr,
}

impl ExprStr {
    /// Parse a source string into an `ExprStr`. The `raw` form is the
    /// caller-supplied input, untouched.
    pub fn from_source(src: &str) -> Result<Self, ParseError> {
        let parsed = parse(src)?;
        Ok(Self {
            raw: src.to_string(),
            parsed,
        })
    }
}

impl PartialEq for ExprStr {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl Eq for ExprStr {}

impl fmt::Display for ExprStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl Serialize for ExprStr {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for ExprStr {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(de)?;
        let parsed = parse(&raw).map_err(serde::de::Error::custom)?;
        Ok(Self { raw, parsed })
    }
}

/// The closed set of expression node kinds at v0.1.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Lit(Literal),
    Path(Path),
    Call {
        path: Path,
        args: Vec<Expr>,
    },
    BinOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        expr: Box<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

/// `$<root>.<segments...>` — the only path form at v0.1. Roots are a
/// closed enum so a stray `$foo.bar` fails parse, not validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub root: PathRoot,
    pub segments: Vec<String>,
}

impl Path {
    /// Walk the segments after the root. Convenience for the validator.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRoot {
    /// `$steps.<step_id>.outputs.<output_name>` — bound by the
    /// declaring check.
    Steps,
    /// `$setup.<step_id>.outputs.<output_name>` — bound by the
    /// Verification Definition's run-level `setup:` block. Run-scoped
    /// and read-only from inside any check (per issue #20).
    Setup,
    /// `$inputs.<input_name>` — bound by the Verification Definition's
    /// `inputs:` block.
    Inputs,
    /// `$env.<name>` — whitelisted environment variables. The schema
    /// crate treats the catalog as open; the runtime spec owns the
    /// whitelist.
    Env,
    /// `$runtime.<fn>(...)` — built-in helpers exposed by the runtime
    /// (e.g. `uuid()`, `now()`). The schema doesn't enumerate the
    /// catalog; the runtime spec owns it. Calls (`(...)` suffix) are
    /// only legal under this root.
    Runtime,
}

impl PathRoot {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steps => "steps",
            Self::Setup => "setup",
            Self::Inputs => "inputs",
            Self::Env => "env",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
}

impl Expr {
    /// Walk every `Path` reachable from this expression. Used by the
    /// validator to resolve `$steps.X.outputs.Y` and `$inputs.X`
    /// references against the declared steps/inputs.
    pub fn walk_paths<F: FnMut(&Path)>(&self, mut visit: F) {
        fn go<F: FnMut(&Path)>(e: &Expr, visit: &mut F) {
            match e {
                Expr::Lit(_) => {}
                Expr::Path(p) => visit(p),
                Expr::Call { path, args } => {
                    visit(path);
                    for a in args {
                        go(a, visit);
                    }
                }
                Expr::BinOp { lhs, rhs, .. } => {
                    go(lhs, visit);
                    go(rhs, visit);
                }
                Expr::UnaryOp { expr, .. } => go(expr, visit),
            }
        }
        go(self, &mut visit);
    }
}
