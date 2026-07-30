//! Input shape to the judge: per-assertion outcomes the runtime
//! produces by evaluating each `Assertion` (`duhem-schema`) against
//! observed state.
//!
//! The judge consumes these — it does not produce them. Evaluation of
//! an `Assertion` against a step trace is the runtime's job (see
//! `spec(runtime): expression evaluator`). Keeping the boundary here
//! is the structural firewall for the asymmetric-trust commitment
//! (`docs/duhem-spec.md` §11.2): the runtime makes claims, the judge
//! aggregates them, and the two halves can be authored independently.

use serde::{Deserialize, Serialize};

use crate::verdict::VerdictState;

/// One assertion's evaluated state.
///
/// `assertion_index` points back into the source check's
/// `assertions: Vec<Assertion>` (per `duhem-schema`). The judge does
/// not need the assertion itself — it only aggregates the outcome —
/// but the index travels through to evidence so a `fail` can be
/// rendered against the human-authored line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssertionOutcome {
    pub assertion_index: usize,
    pub state: VerdictState,
    /// Human-readable, evidence-bound. Per §8, this is *never*
    /// structured-causal: it explains, it does not localize blame
    /// inside the web. The judge is opaque to its contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One check's input to the judge: the assertion-outcome vector,
/// plus the check id for evidence-side rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckOutcome {
    pub check_id: String,
    pub assertions: Vec<AssertionOutcome>,
}
