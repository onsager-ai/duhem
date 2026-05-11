//! Check executor — orchestrates step execution against the real
//! delivery web.
//!
//! Per `docs/duhem-spec.md` §8 (Holistic Verification Principle): a
//! check exercises code + prompts + tool wiring + data + runtime
//! together. The runtime drives that exercise, hands raw step
//! outcomes to the judge, and writes evidence as it goes.
//!
//! Synchronous in-process for now — runtime → judge → evidence is one
//! call path, not a bus topology. The minimal step executor lands in a
//! follow-up spec (`spec(runtime): minimal step executor`).

pub mod eval;

pub use eval::{EvalContext, EvalResult, InconclusiveCause, Value, ValueShape, eval};
