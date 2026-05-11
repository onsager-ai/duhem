//! Check executor — orchestrates step execution against the real
//! delivery web.
//!
//! Per `docs/duhem-spec.md` §8 (Holistic Verification Principle): a
//! check exercises code + prompts + tool wiring + data + runtime
//! together. The runtime drives that exercise, hands raw step
//! outcomes to the judge, and writes evidence as it goes.
//!
//! Synchronous in-process: runtime → judge → evidence is one call
//! path, not a bus topology. The v1 minimal step executor lives in
//! [`engine`]; the expression evaluator it drives lives in [`eval`].

pub mod engine;
pub mod eval;

pub use engine::{Engine, EngineError, RunContext, RunState, assertion_to_expr};
pub use eval::{EvalContext, EvalResult, InconclusiveCause, Value, ValueShape, eval};
