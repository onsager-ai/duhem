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

use std::fmt;

use serde::de::{Error, Unexpected};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::verdict::{InconclusiveCause, VerdictState};

/// One assertion's state before check aggregation. `Skipped` is the
/// absence of a verdict: [`aggregate_check`](crate::aggregate_check)
/// discards it before producing Duhem's ordinary three-state verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertionState {
    Pass,
    Fail,
    Inconclusive(InconclusiveCause),
    Skipped,
}

impl AssertionState {
    pub fn as_verdict(self) -> Option<VerdictState> {
        match self {
            Self::Pass => Some(VerdictState::Pass),
            Self::Fail => Some(VerdictState::Fail),
            Self::Inconclusive(cause) => Some(VerdictState::Inconclusive(cause)),
            Self::Skipped => None,
        }
    }
}

impl From<VerdictState> for AssertionState {
    fn from(value: VerdictState) -> Self {
        match value {
            VerdictState::Pass => Self::Pass,
            VerdictState::Fail => Self::Fail,
            VerdictState::Inconclusive(cause) => Self::Inconclusive(cause),
        }
    }
}

impl fmt::Display for AssertionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => f.write_str("pass"),
            Self::Fail => f.write_str("fail"),
            Self::Inconclusive(cause) => write!(f, "inconclusive:{cause}"),
            Self::Skipped => f.write_str("skipped"),
        }
    }
}

impl Serialize for AssertionState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AssertionState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            "pass" => Ok(Self::Pass),
            "fail" => Ok(Self::Fail),
            "skipped" => Ok(Self::Skipped),
            other => match other.strip_prefix("inconclusive:") {
                Some(cause) => InconclusiveCause::from_wire(cause)
                    .map(Self::Inconclusive)
                    .ok_or_else(|| {
                        D::Error::invalid_value(
                            Unexpected::Str(other),
                            &"a known inconclusive cause",
                        )
                    }),
                None => Err(D::Error::invalid_value(
                    Unexpected::Str(other),
                    &"pass, fail, skipped, or inconclusive:<cause>",
                )),
            },
        }
    }
}

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
    pub state: AssertionState,
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
