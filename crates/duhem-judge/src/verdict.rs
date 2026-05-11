//! `VerdictState` and its inconclusive causes.
//!
//! Per `docs/duhem-spec.md` §7.6, the verdict surface is three-state:
//! `pass`, `fail`, `inconclusive`. `Inconclusive` carries a cause so
//! evidence can distinguish "the artifact is broken" (which is `fail`)
//! from "we couldn't tell" (timeouts, missing observations, the
//! runtime failed to provision). This distinction is the whole point
//! of three-state — collapsing `inconclusive` into `fail` would
//! double-count flaky infra as artifact defects.
//!
//! The variant set on `VerdictState` is doctrinally closed (§7.6 calls
//! three-state non-negotiable), so it is not `#[non_exhaustive]`.
//! `InconclusiveCause` *is* `#[non_exhaustive]` so future causes can
//! be added without a breaking change at the type level — though
//! adding a wire value is still a schema-impact event.

use std::fmt;

use serde::de::{Error as DeError, Unexpected};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The three-state verdict. See `docs/duhem-spec.md` §7.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictState {
    Pass,
    Fail,
    Inconclusive(InconclusiveCause),
}

/// Why a verdict came out `inconclusive`. Closed at v1 per the
/// alignment ratification on the spec issue; growth is additive via
/// `#[non_exhaustive]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InconclusiveCause {
    /// A step or assertion exceeded its `within` budget.
    Timeout,
    /// An assertion referenced a step output that was never produced.
    MissingObservation,
    /// The runtime failed to provision the environment; the artifact
    /// was never exercised, so its verdict is unknown.
    EnvironmentError,
    /// Defensive: aggregation called on an empty vector. The schema
    /// validator forbids empty `assertions` / `checks` / `criteria`,
    /// so this should be unreachable in production runs; the judge
    /// returns it instead of panicking when the invariant is broken.
    EmptyAggregation,
}

impl InconclusiveCause {
    /// Stable wire token. Snake-case is the convention across the
    /// rest of the schema (`docs/duhem-spec.md` §10).
    pub fn as_wire(self) -> &'static str {
        match self {
            InconclusiveCause::Timeout => "timeout",
            InconclusiveCause::MissingObservation => "missing_observation",
            InconclusiveCause::EnvironmentError => "environment_error",
            InconclusiveCause::EmptyAggregation => "empty_aggregation",
        }
    }

    fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "timeout" => InconclusiveCause::Timeout,
            "missing_observation" => InconclusiveCause::MissingObservation,
            "environment_error" => InconclusiveCause::EnvironmentError,
            "empty_aggregation" => InconclusiveCause::EmptyAggregation,
            _ => return None,
        })
    }
}

impl fmt::Display for InconclusiveCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl fmt::Display for VerdictState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerdictState::Pass => f.write_str("pass"),
            VerdictState::Fail => f.write_str("fail"),
            VerdictState::Inconclusive(cause) => write!(f, "inconclusive:{cause}"),
        }
    }
}

impl Serialize for VerdictState {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for VerdictState {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        match s.as_str() {
            "pass" => Ok(VerdictState::Pass),
            "fail" => Ok(VerdictState::Fail),
            other => match other.strip_prefix("inconclusive:") {
                Some(cause) => InconclusiveCause::from_wire(cause)
                    .map(VerdictState::Inconclusive)
                    .ok_or_else(|| {
                        D::Error::invalid_value(
                            Unexpected::Str(other),
                            &"a known inconclusive cause \
                              (timeout, missing_observation, environment_error, empty_aggregation)",
                        )
                    }),
                None => Err(D::Error::invalid_value(
                    Unexpected::Str(other),
                    &"\"pass\", \"fail\", or \"inconclusive:<cause>\"",
                )),
            },
        }
    }
}
