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

/// Aggregated verdict across all leaves of a root-manifest run. Same
/// three-state rule as within a leaf (issue #49 § "Run semantics
/// across leaves"): any `Fail` → `Fail`; else any `Inconclusive` →
/// `Inconclusive`; else `Pass`. Carries the child `RunVerdict`s so the
/// CLI / reporters can render per-leaf detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSetVerdict {
    pub state: VerdictState,
    pub runs: Vec<RunVerdict>,
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

/// How a criterion-level `inconclusive` verdict is treated at run
/// aggregation (manifest `defaults.inconclusive_policy`, spec #66).
///
/// The judge owns the *meaning* of the policy; the schema owns its
/// wire shape (`duhem_schema::InconclusivePolicy`). The runtime maps
/// one to the other so this crate stays free of a schema dependency —
/// the mechanical-judgment firewall (§11.2) keeps the verdict logic
/// shippable on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InconclusivePolicy {
    /// Today's behavior: a criterion-level `inconclusive` stays
    /// `inconclusive` and does not pass.
    #[default]
    Block,
    /// Treat a criterion-level `inconclusive` as a pass, but emit a
    /// warning so the run summary can surface it.
    Warn,
    /// Silently treat a criterion-level `inconclusive` as a pass.
    Pass,
}

/// Apply `policy` to a single criterion's aggregated verdict.
///
/// Returns the (possibly softened) state and an optional warning. Only
/// `Inconclusive` is ever softened, and only under `warn` / `pass`:
///
/// - `block` — pass-through; `inconclusive` stays `inconclusive`
///   (today's behavior). No warning.
/// - `warn` — `inconclusive` → `Pass`, with a warning naming the
///   criterion and the cause so the run summary surfaces it.
/// - `pass` — `inconclusive` → `Pass`, silently (no warning).
///
/// `Pass` and `Fail` are never touched — the policy governs only the
/// "we couldn't tell" verdict, never a real defect. Per-assertion
/// evaluation upstream is unchanged; this is a criterion-level lens.
pub fn apply_inconclusive_policy(
    criterion_id: &str,
    state: VerdictState,
    policy: InconclusivePolicy,
) -> (VerdictState, Option<String>) {
    match (state, policy) {
        (VerdictState::Inconclusive(cause), InconclusivePolicy::Warn) => (
            VerdictState::Pass,
            Some(format!(
                "criterion {criterion_id}: inconclusive ({cause}) treated as pass by inconclusive_policy: warn"
            )),
        ),
        (VerdictState::Inconclusive(_), InconclusivePolicy::Pass) => (VerdictState::Pass, None),
        // `block` (any state) and `warn`/`pass` over a non-inconclusive
        // state pass through untouched.
        (other, _) => (other, None),
    }
}

/// Roll up the per-leaf `RunVerdict`s of a manifest run. Same
/// three-state rule and same first-inconclusive-wins ordering as
/// every other level — re-aggregation is identity-preserving across
/// the nesting depth (issue #49).
pub fn aggregate_run_set(verdicts: Vec<RunVerdict>) -> RunSetVerdict {
    let state = fold_verdicts(verdicts.iter().map(|c| c.state));
    RunSetVerdict {
        state,
        runs: verdicts,
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

    fn rv(state: VerdictState) -> RunVerdict {
        RunVerdict {
            state,
            criteria: vec![],
        }
    }

    #[test]
    fn aggregate_run_set_three_state_truth_table() {
        // Spec on #49: same rule as `aggregate_run`. Cover the three
        // shapes that matter: all pass, any fail, any inconclusive-no-fail.
        assert_eq!(
            aggregate_run_set(vec![rv(VerdictState::Pass), rv(VerdictState::Pass)]).state,
            VerdictState::Pass,
        );
        assert_eq!(
            aggregate_run_set(vec![rv(VerdictState::Pass), rv(VerdictState::Fail)]).state,
            VerdictState::Fail,
        );
        assert_eq!(
            aggregate_run_set(vec![rv(VerdictState::Pass), rv(TIMEOUT)]).state,
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
        );
        assert_eq!(
            aggregate_run_set(vec![rv(TIMEOUT), rv(VerdictState::Fail)]).state,
            VerdictState::Fail,
            "Fail dominates Inconclusive across leaves",
        );
    }

    #[test]
    fn aggregate_run_set_preserves_child_runs() {
        let set = aggregate_run_set(vec![rv(VerdictState::Pass), rv(VerdictState::Fail)]);
        assert_eq!(set.runs.len(), 2);
        assert_eq!(set.runs[0].state, VerdictState::Pass);
        assert_eq!(set.runs[1].state, VerdictState::Fail);
    }

    #[test]
    fn aggregate_run_set_empty_is_empty_aggregation() {
        let set = aggregate_run_set(vec![]);
        assert_eq!(
            set.state,
            VerdictState::Inconclusive(InconclusiveCause::EmptyAggregation),
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

    // -- inconclusive policy (spec #66) --------------------------------------

    #[test]
    fn policy_block_is_todays_behavior() {
        // `block` never softens — an inconclusive criterion stays
        // inconclusive, and nothing else moves either.
        for state in [
            VerdictState::Pass,
            VerdictState::Fail,
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
            VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
        ] {
            let (out, warn) = apply_inconclusive_policy("AC-1", state, InconclusivePolicy::Block);
            assert_eq!(out, state, "block must pass through {state:?}");
            assert!(warn.is_none(), "block never warns");
        }
    }

    #[test]
    fn policy_warn_softens_inconclusive_to_pass_with_a_warning() {
        let (out, warn) = apply_inconclusive_policy(
            "AC-1",
            VerdictState::Inconclusive(InconclusiveCause::Timeout),
            InconclusivePolicy::Warn,
        );
        assert_eq!(out, VerdictState::Pass);
        let w = warn.expect("warn policy surfaces a warning");
        assert!(w.contains("AC-1"), "warning names the criterion: {w}");
        assert!(w.contains("timeout"), "warning names the cause: {w}");
    }

    #[test]
    fn policy_pass_softens_inconclusive_silently() {
        let (out, warn) = apply_inconclusive_policy(
            "AC-1",
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation),
            InconclusivePolicy::Pass,
        );
        assert_eq!(out, VerdictState::Pass);
        assert!(warn.is_none(), "pass policy is silent");
    }

    #[test]
    fn policy_never_softens_fail() {
        // A real defect (`fail`) survives every policy — the policy
        // governs only "we couldn't tell", never a defect.
        for policy in [
            InconclusivePolicy::Block,
            InconclusivePolicy::Warn,
            InconclusivePolicy::Pass,
        ] {
            let (out, warn) = apply_inconclusive_policy("AC-1", VerdictState::Fail, policy);
            assert_eq!(out, VerdictState::Fail, "fail must survive {policy:?}");
            assert!(warn.is_none());
        }
    }

    #[test]
    fn policy_default_is_block() {
        assert_eq!(InconclusivePolicy::default(), InconclusivePolicy::Block);
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
