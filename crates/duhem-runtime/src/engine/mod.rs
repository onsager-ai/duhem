//! `duhem run` step executor.
//!
//! Integrator that turns the substrate from the upstream specs into
//! an end-to-end `duhem run` (issue #15). Per the spec, this module
//! owns lifecycle and dispatch — it does not own grammar, judge
//! folds, action implementations, or evidence wire format. Those
//! belong to `duhem-schema`, `duhem-judge`, `duhem-actions`, and
//! `duhem-evidence` respectively; the engine is purely compositional.

pub mod context;
pub mod registry;
pub mod runner;
pub mod shim;
pub mod template;

pub use context::{RunContext, RunState};
pub use runner::{Engine, EngineError};
pub use shim::assertion_to_expr;
