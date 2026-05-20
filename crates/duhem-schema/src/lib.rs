//! Verification Definition types and validator.
//!
//! Owns the on-the-wire shape of a Verification Definition — criteria
//! (the human commitment about what "done" means) and checks (the
//! mechanically-judged assertions that verify it). Per
//! `docs/duhem-spec.md` §7.2 / §7.3, criteria are stable; checks are
//! derivative.

pub mod assertion;
pub mod criterion;
pub mod environment;
pub mod expr;
pub mod step;
pub mod validate;
pub mod verification;

pub use assertion::{Assertion, TypeCheckKind};
pub use criterion::{Check, Criterion};
pub use environment::{DurationSpec, Environment, HttpReadyProbe, ReadyProbe};
pub use expr::{BinOp, Expr, ExprStr, Literal, ParseError, Path, PathRoot, UnaryOp};
pub use step::Step;
pub use validate::{ValidationError, validate};
pub use verification::{InputDecl, InputType, SchemaError, VerificationDefinition};
