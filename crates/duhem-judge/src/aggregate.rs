//! Verdict aggregation. Pure, deterministic, identical rules at every
//! level.
//!
//! Rules (verbatim, `docs/duhem-spec.md` §7.6):
//!
//! - any `Fail` → `Fail`
//! - any `Inconclusive` and no `Fail` → `Inconclusive`
//! - all `Pass` → `Pass`
//!
//! When multiple `Inconclusive` children are present, the *first*
//! observed cause is carried up. First-wins matches evidence-trace
//! ordering and is the simplest rule that round-trips through a
//! single-pass aggregator — confirmed in the spec-issue alignment.
//!
//! Empty input is treated as `Inconclusive(EmptyAggregation)`. The
//! schema validator (`duhem-schema::validate`) rejects empty
//! `assertions` / `checks` / `criteria`, so a well-formed run never
//! hits this case; the judge handles it defensively rather than
//! panicking when the invariant has been violated upstream.

use serde::{Deserialize, Serialize};

use crate::outcome::CheckOutcome;
use crate::verdict::{InconclusiveCause, VerdictState};

/// One check's verdict. Output of `aggregate_check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckVerdict {
    pub check_id: String,
    pub state: VerdictState,
}

/// One criterion's verdict bundle. Caller composes `criterion_id` +
/// child `CheckVerdict`s; `state` is `aggregate_criterion(&checks)`.
/// The struct is *data*, not a constructor — it carries the children
/// for evidence rendering, which is why `aggregate_criterion` only
/// returns the state and not this struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriterionVerdict {
    pub criterion_id: String,
    pub state: VerdictState,
    pub checks: Vec<CheckVerdict>,
}

/// Top-level verdict for one `duhem run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunVerdict {
    pub state: VerdictState,
    pub criteria: Vec<CriterionVerdict>,
}

/// The single shared aggregation step. `Iterator` not `slice` so
/// callers can fold without materializing intermediate vectors
/// (e.g. mapping `CheckVerdict.state` directly).
fn fold_verdicts<I>(items: I) -> VerdictState
where
    I: IntoIterator<Item = VerdictState>,
{
    let mut first_inconclusive: Option<InconclusiveCause> = None;
    let mut seen_fail = false;
    let mut count: usize = 0;
    for v in items {
        count += 1;
        match v {
            VerdictState::Fail => seen_fail = true,
            VerdictState::Inconclusive(cause) => {
                if first_inconclusive.is_none() {
                    first_inconclusive = Some(cause);
                }
            }
            VerdictState::Pass => {}
        }
    }
    if count == 0 {
        return VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation);
    }
    if seen_fail {
        VerdictState::Fail
    } else if let Some(cause) = first_inconclusive {
        VerdictState::Inconclusive(cause)
    } else {
        VerdictState::Pass
    }
}

/// Roll up one check's per-assertion outcomes into a `CheckVerdict`.
pub fn aggregate_check(outcome: &CheckOutcome) -> CheckVerdict {
    let state = fold_verdicts(outcome.assertions.iter().map(|a| a.state));
    CheckVerdict {
        check_id: outcome.check_id.clone(),
        state,
    }
}

/// Roll up one criterion's child check verdicts into a state.
/// Returns just the state — caller pairs it with `criterion_id` and
/// the source `checks` when building a `CriterionVerdict`.
pub fn aggregate_criterion(verdicts: &[CheckVerdict]) -> VerdictState {
    fold_verdicts(verdicts.iter().map(|c| c.state))
}

/// Roll up all criteria into a `RunVerdict`. Consumes the children
/// so the caller does not have to clone them — they're already the
/// canonical record for evidence.
///
/// **Empty input.** `aggregate_run(vec![])` is defined as
/// `Inconclusive(EmptyAggregation)`. Before the setup-step spec
/// (issue #20) the engine never called it with no criteria — the
/// schema validator forbids empty `criteria`. Setup execution
/// changes that: when run-level setup aborts under §10.3's failure
/// policy, no criterion executes and the run finishes with an empty
/// criterion vector. Returning `Inconclusive` here is the
/// three-state-faithful answer — we couldn't observe the workload
/// in the state the Verification Definition claims to verify, so
/// the verdict is "we don't know," not "fail."
pub fn aggregate_run(verdicts: Vec<CriterionVerdict>) -> RunVerdict {
    let state = fold_verdicts(verdicts.iter().map(|c| c.state));
    RunVerdict {
        state,
        criteria: verdicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::AssertionOutcome;

    fn ao(state: VerdictState) -> AssertionOutcome {
        AssertionOutcome {
            assertion_index: 0,
            state,
            detail: None,
        }
    }

    fn cv(state: VerdictState) -> CheckVerdict {
        CheckVerdict {
            check_id: "c".into(),
            state,
        }
    }

    fn crv(state: VerdictState) -> CriterionVerdict {
        CriterionVerdict {
            criterion_id: "AC-1".into(),
            state,
            checks: vec![],
        }
    }

    // -- truth table over the three states ------------------------------------

    const TIMEOUT: VerdictState = VerdictState::Inconclusive(InconclusiveCause::Timeout);

    /// Generate every triple of {Pass, Fail, Inconclusive(Timeout)} —
    /// 27 inputs — and confirm the rule against a hand-coded oracle.
    #[test]
    fn fold_truth_table_27() {
        let alphabet = [VerdictState::Pass, VerdictState::Fail, TIMEOUT];
        for a in alphabet {
            for b in alphabet {
                for c in alphabet {
                    let xs = [a, b, c];
                    let got = fold_verdicts(xs);
                    let want = oracle(&xs);
                    assert_eq!(got, want, "input {xs:?}");
                }
            }
        }
    }

    fn oracle(xs: &[VerdictState]) -> VerdictState {
        if xs.is_empty() {
            return VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation);
        }
        if xs.iter().any(|v| matches!(v, VerdictState::Fail)) {
            return VerdictState::Fail;
        }
        for v in xs {
            if let VerdictState::Inconclusive(cause) = v {
                return VerdictState::Inconclusive(*cause);
            }
        }
        VerdictState::Pass
    }

    // -- cause-stability ------------------------------------------------------

    #[test]
    fn first_inconclusive_cause_wins() {
        let xs = [
            VerdictState::Pass,
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation),
            VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
        ];
        assert_eq!(
            fold_verdicts(xs),
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
        );
    }

    #[test]
    fn fail_dominates_inconclusive_regardless_of_order() {
        let a = [
            VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
            VerdictState::Fail,
        ];
        let b = [
            VerdictState::Fail,
            VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
        ];
        assert_eq!(fold_verdicts(a), VerdictState::Fail);
        assert_eq!(fold_verdicts(b), VerdictState::Fail);
    }

    // -- empty input ----------------------------------------------------------

    #[test]
    fn empty_check_yields_empty_aggregation() {
        let out = CheckOutcome {
            check_id: "c1".into(),
            assertions: vec![],
        };
        assert_eq!(
            aggregate_check(&out),
            CheckVerdict {
                check_id: "c1".into(),
                state: VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation),
            },
        );
    }

    #[test]
    fn empty_criterion_yields_empty_aggregation() {
        assert_eq!(
            aggregate_criterion(&[]),
            VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation),
        );
    }

    #[test]
    fn empty_run_yields_empty_aggregation() {
        let r = aggregate_run(vec![]);
        assert_eq!(
            r.state,
            VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation),
        );
        assert!(r.criteria.is_empty());
    }

    // -- per-level wiring -----------------------------------------------------

    #[test]
    fn aggregate_check_pass() {
        let out = CheckOutcome {
            check_id: "c1".into(),
            assertions: vec![ao(VerdictState::Pass), ao(VerdictState::Pass)],
        };
        assert_eq!(aggregate_check(&out).state, VerdictState::Pass);
    }

    #[test]
    fn aggregate_check_fail() {
        let out = CheckOutcome {
            check_id: "c1".into(),
            assertions: vec![ao(VerdictState::Pass), ao(VerdictState::Fail)],
        };
        assert_eq!(aggregate_check(&out).state, VerdictState::Fail);
    }

    #[test]
    fn aggregate_check_inconclusive() {
        let out = CheckOutcome {
            check_id: "c1".into(),
            assertions: vec![ao(VerdictState::Pass), ao(TIMEOUT)],
        };
        assert_eq!(
            aggregate_check(&out).state,
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
        );
    }

    #[test]
    fn aggregate_criterion_rolls_up_checks() {
        assert_eq!(
            aggregate_criterion(&[cv(VerdictState::Pass), cv(VerdictState::Pass)]),
            VerdictState::Pass,
        );
        assert_eq!(
            aggregate_criterion(&[cv(VerdictState::Pass), cv(VerdictState::Fail)]),
            VerdictState::Fail,
        );
        assert_eq!(
            aggregate_criterion(&[cv(VerdictState::Pass), cv(TIMEOUT)]),
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
        );
    }

    #[test]
    fn aggregate_run_rolls_up_criteria() {
        let r = aggregate_run(vec![crv(VerdictState::Pass), crv(VerdictState::Pass)]);
        assert_eq!(r.state, VerdictState::Pass);
        assert_eq!(r.criteria.len(), 2);

        let r = aggregate_run(vec![crv(VerdictState::Pass), crv(VerdictState::Fail)]);
        assert_eq!(r.state, VerdictState::Fail);

        let r = aggregate_run(vec![crv(VerdictState::Pass), crv(TIMEOUT)]);
        assert_eq!(
            r.state,
            VerdictState::Inconclusive(InconclusiveCause::Timeout)
        );
    }

    // -- re-aggregation identity ---------------------------------------------

    /// The spec's named property: folding a sub-fold equals folding the
    /// flattened input. We assert it for cases where order-of-first-
    /// inconclusive is preserved by left-associative grouping.
    #[test]
    fn nested_fold_equals_flat_fold() {
        let cases: &[&[VerdictState]] = &[
            &[VerdictState::Pass, VerdictState::Fail],
            &[VerdictState::Pass, VerdictState::Pass],
            &[VerdictState::Pass, TIMEOUT],
            &[VerdictState::Fail, VerdictState::Fail],
            &[TIMEOUT, VerdictState::Fail],
        ];
        for xs in cases {
            let flat = fold_verdicts(xs.iter().copied());
            // wrap the tail in its own fold first
            let head = xs[0];
            let tail = fold_verdicts(xs[1..].iter().copied());
            let nested = fold_verdicts([head, tail]);
            assert_eq!(flat, nested, "for {xs:?}");
        }
    }

    // -- JSON round-trip ------------------------------------------------------

    #[test]
    fn verdict_state_round_trips() {
        for v in [
            VerdictState::Pass,
            VerdictState::Fail,
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation),
            VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
            VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation),
        ] {
            let s = serde_json::to_string(&v).unwrap();
            let back: VerdictState = serde_json::from_str(&s).unwrap();
            assert_eq!(v, back, "round-trip via {s}");
        }
    }

    #[test]
    fn verdict_state_wire_strings() {
        assert_eq!(
            serde_json::to_string(&VerdictState::Pass).unwrap(),
            "\"pass\"",
        );
        assert_eq!(
            serde_json::to_string(&VerdictState::Fail).unwrap(),
            "\"fail\"",
        );
        assert_eq!(
            serde_json::to_string(&VerdictState::Inconclusive(InconclusiveCause::Timeout)).unwrap(),
            "\"inconclusive:timeout\"",
        );
    }

    #[test]
    fn verdict_state_rejects_unknown_strings() {
        assert!(serde_json::from_str::<VerdictState>("\"maybe\"").is_err());
        assert!(serde_json::from_str::<VerdictState>("\"inconclusive:wat\"").is_err());
        assert!(serde_json::from_str::<VerdictState>("\"inconclusive\"").is_err());
    }
}
