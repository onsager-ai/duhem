//! `duhem run` step executor.
//!
//! Integrator that turns the substrate from the upstream specs into
//! an end-to-end `duhem run` (issue #15). Per the spec, this module
//! owns lifecycle and dispatch — it does not own grammar, judge
//! folds, action implementations, or evidence wire format. Those
//! belong to `duhem-schema`, `duhem-judge`, `duhem-actions`, and
//! `duhem-evidence` respectively; the engine is purely compositional.

pub mod capture;
pub mod context;
pub mod env;
pub mod identity;
pub mod inherit;
pub mod outcome;
pub mod registry;
pub mod runner;
pub mod setup;
pub mod shim;
pub mod template;
pub mod translate;

pub use capture::CapturePolicy;
pub use context::{RunContext, RunState};
pub use env::SuiteEnvironment;
pub use identity::resolve_scope;
pub use runner::{
    CapturedArtifact, CheckFailure, CheckFilter, Engine, EngineError, FailedAssertion, RunOutcome,
};
