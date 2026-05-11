//! Deterministic verdict aggregator for verification check results.
//!
//! Per `docs/duhem-spec.md` §7.6 / §11.2: judgment is mechanical, not
//! LLM-driven. Given a vector of structured assertion outcomes from
//! the runtime, produce a `pass` / `fail` / `inconclusive` verdict
//! by pure deterministic evaluation. No model in the loop, ever.
//!
//! The crate is types-in / types-out: it does not own evaluation of
//! `Assertion`s (that's the runtime's job — see `spec(runtime):
//! expression evaluator`), it does not own evidence serialization
//! (that's `duhem-evidence`), and it does not own override / escalation
//! policy (§9 Stage 5). It owns only the aggregation rules and the
//! verdict wire shape.
//!
//! Carved off as a separate crate from day one (per §11.2 OSS-judge
//! boundary) so the future open-source reference judge can ship without
//! pulling in runtime / actions / evidence machinery. Cargo manifest
//! deliberately depends on no HTTP client and no AI SDK; this is the
//! structural firewall behind the mechanical-judgment commitment.

pub mod aggregate;
pub mod outcome;
pub mod verdict;

pub use aggregate::{
    CheckVerdict, CriterionVerdict, RunVerdict, aggregate_check, aggregate_criterion, aggregate_run,
};
pub use outcome::{AssertionOutcome, CheckOutcome};
pub use verdict::{InconclusiveCause, VerdictState};
