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

pub use engine::{
    CapturePolicy, CapturedArtifact, CheckFailure, CheckFilter, Engine, EngineError,
    FailedAssertion, RunContext, RunOutcome, RunState, SuiteEnvironment, resolve_scope,
};
pub use eval::{EvalContext, EvalResult, InconclusiveCause, Value, ValueShape, eval};

// `assertion_to_expr` is deliberately not re-exported at the crate
// root — it's a runtime-internal shim per its module docs and the
// spec on issue #15. In-crate callers reach it via
// `crate::engine::shim::assertion_to_expr`.
