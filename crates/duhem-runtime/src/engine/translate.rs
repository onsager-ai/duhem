//! Pure translation + policy helpers for the step executor.
//!
//! Carved out of `runner.rs` so the lifecycle code there stays under
//! the per-file token budget. Everything here is a small, side-effect-
//! free mapping: evaluator results → verdict states, evaluator causes →
//! evidence-detail strings, action outcomes → evidence outcomes, and
//! the `defaults:` (#66) timeout-injection + retry-classification
//! rules. No I/O, no `Engine` state.

use std::collections::BTreeMap;
use std::time::Duration;

use duhem_actions::Outcome;
use duhem_evidence::{StepOutcome, VerdictState};
use duhem_judge::InconclusiveCause;
use duhem_schema::RetryBackoff;

use crate::eval::{EvalResult, InconclusiveCause as EvalCause};

/// Production base delay between check retries (spec #66). The first
/// retry waits this long; subsequent retries scale it per the chosen
/// [`RetryBackoff`] schedule.
pub(super) const RETRY_BACKOFF_BASE: Duration = Duration::from_millis(500);

/// Whether a check verdict is retry-eligible (spec #66, matching #54's
/// classification). Only an `inconclusive` for a *recoverable* cause
/// retries: a timeout or an environment error. A `fail` is a real
/// defect and never retries; `missing_observation` /
/// `empty_aggregation` reflect the Verification Definition's own shape,
/// not flaky infra, so they don't retry either. `pass` never retries.
pub(super) fn check_is_retryable(state: VerdictState) -> bool {
    matches!(
        state,
        VerdictState::Inconclusive(InconclusiveCause::Timeout)
            | VerdictState::Inconclusive(InconclusiveCause::EnvironmentError)
    )
}

/// Delay before the `attempt`-th retry (1-based) under `backoff`,
/// scaled from `base`. Linear: `base · attempt`. Exponential:
/// `base · 2^(attempt-1)`. Saturating so a large `max` can't overflow.
pub(super) fn retry_delay(base: Duration, backoff: RetryBackoff, attempt: u32) -> Duration {
    let factor: u32 = match backoff {
        RetryBackoff::Linear => attempt,
        RetryBackoff::Exponential => 1u32
            .checked_shl(attempt.saturating_sub(1))
            .unwrap_or(u32::MAX),
    };
    base.saturating_mul(factor)
}

/// Fill a step's `within:` from the manifest `defaults.timeout` when
/// it doesn't already declare one (spec #66). Only a `with:` that's a
/// mapping is touched, and only when it lacks a `within` key — so a
/// per-step `within:` always wins. The value is written as integer
/// milliseconds, the form `duhem_actions::WithinSpec` accepts. A
/// duration past `u64::MAX` ms is left out rather than truncated (the
/// action's `DEFAULT_WITHIN` then applies); such a value is not
/// reachable from the `DurationSpec` wire shape in practice.
pub(super) fn apply_default_within(with: &mut serde_yml::Value, default: Duration) {
    let ms = default.as_millis();
    if ms > u64::MAX as u128 {
        return;
    }
    if let serde_yml::Value::Mapping(m) = with {
        let key = serde_yml::Value::String("within".to_string());
        if !m.contains_key(&key) {
            m.insert(key, serde_yml::Value::Number((ms as u64).into()));
        }
    }
}

pub(super) fn eval_to_state(r: &EvalResult) -> VerdictState {
    match r {
        EvalResult::True => VerdictState::Pass,
        EvalResult::False => VerdictState::Fail,
        // A type mismatch is an authoring defect: the assertion applied an
        // operator to the wrong value shape (e.g. `contains(str, array)`,
        // `len(int)`). A retry can't fix it and the environment is fine, so
        // it gates as `fail` — blocking the gate and naming the specific
        // `type_mismatch(...)` in the evidence detail — rather than a
        // retry-eligible, misleading `inconclusive:environment_error`
        // (#259). Other malformed-expression causes keep their existing
        // classification for now.
        EvalResult::Inconclusive(EvalCause::TypeMismatch { .. }) => VerdictState::Fail,
        EvalResult::Inconclusive(cause) => VerdictState::Inconclusive(map_eval_cause(cause)),
    }
}

/// Coarsen an evaluator cause to the judge's `InconclusiveCause` set.
/// `TypeMismatch` is intercepted as `fail` in `eval_to_state` and so
/// never reaches its arm below (kept for exhaustiveness).
fn map_eval_cause(c: &EvalCause) -> InconclusiveCause {
    match c {
        EvalCause::MissingObservation { .. }
        | EvalCause::MissingSetupObservation { .. }
        | EvalCause::MissingInput(_)
        | EvalCause::MissingEnv(_)
        | EvalCause::MissingField(_) => InconclusiveCause::MissingObservation,
        EvalCause::UnknownRuntimeHelper(_)
        | EvalCause::TypeMismatch { .. }
        | EvalCause::NotNavigable { .. }
        | EvalCause::BadFormat(_)
        | EvalCause::InvalidPattern(_) => InconclusiveCause::EnvironmentError,
    }
}

/// Format an evaluator-level `InconclusiveCause` for the
/// evidence-side `detail` field. The judge's `VerdictState` cause set
/// is intentionally coarse (`MissingObservation` /
/// `EnvironmentError` / etc.); the evidence-side string preserves the
/// specific reason so a reader can distinguish `missing_input(x)`
/// from `missing_observation(api.body)`, or
/// `invalid_pattern(...)` from `type_mismatch(str,int)`.
pub(super) fn eval_cause_detail(c: &EvalCause) -> String {
    match c {
        EvalCause::MissingObservation { step, output } => {
            format!("missing_observation({step}.{output})")
        }
        EvalCause::MissingSetupObservation { step, output } => {
            format!("missing_setup_observation({step}.{output})")
        }
        EvalCause::MissingInput(n) => format!("missing_input({n})"),
        EvalCause::MissingEnv(n) => format!("missing_env({n})"),
        EvalCause::UnknownRuntimeHelper(n) => format!("unknown_runtime_helper({n})"),
        EvalCause::TypeMismatch { lhs, rhs } => {
            format!("type_mismatch({}, {})", shape_wire(*lhs), shape_wire(*rhs))
        }
        EvalCause::InvalidPattern(msg) => format!("invalid_pattern({msg})"),
        EvalCause::MissingField(path) => format!("missing_field({path})"),
        EvalCause::NotNavigable { shape, segment } => {
            format!("not_navigable({}, {segment})", shape_wire(*shape))
        }
        EvalCause::BadFormat(msg) => format!("bad_format({msg})"),
    }
}

fn shape_wire(s: crate::eval::ValueShape) -> &'static str {
    use crate::eval::ValueShape;
    match s {
        ValueShape::Bool => "bool",
        ValueShape::Int => "int",
        ValueShape::Float => "float",
        ValueShape::Str => "str",
        ValueShape::Null => "null",
        ValueShape::Array => "array",
        ValueShape::Object => "object",
    }
}

pub(crate) fn outcome_to_evidence(o: &Outcome) -> StepOutcome {
    match o {
        Outcome::Ok => StepOutcome::Ok,
        Outcome::Error => StepOutcome::Error,
        Outcome::Timeout => StepOutcome::Timeout,
    }
}

pub(crate) fn with_to_evidence_map(v: &serde_yml::Value) -> BTreeMap<String, serde_json::Value> {
    match v {
        serde_yml::Value::Mapping(m) => m
            .iter()
            .filter_map(|(k, v)| {
                let key = k.as_str()?.to_string();
                let val = yml_to_json(v);
                Some((key, val))
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn yml_to_json(v: &serde_yml::Value) -> serde_json::Value {
    use serde_yml::Value as Y;
    match v {
        Y::Null => serde_json::Value::Null,
        Y::Bool(b) => serde_json::Value::Bool(*b),
        Y::Number(n) => serde_json::to_value(n).unwrap_or(serde_json::Value::Null),
        Y::String(s) => serde_json::Value::String(s.clone()),
        Y::Sequence(seq) => serde_json::Value::Array(seq.iter().map(yml_to_json).collect()),
        Y::Mapping(m) => serde_json::Value::Object(
            m.iter()
                .filter_map(|(k, v)| {
                    let key = k.as_str()?.to_string();
                    Some((key, yml_to_json(v)))
                })
                .collect(),
        ),
        Y::Tagged(t) => yml_to_json(&t.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_mismatch_gates_as_fail_not_environment_error() {
        // A type mismatch is a defect in the VD, not flaky infra: it must
        // gate as `fail`, never a retry-eligible
        // `inconclusive:environment_error` (#259).
        use crate::eval::ValueShape;
        let state = eval_to_state(&EvalResult::Inconclusive(EvalCause::TypeMismatch {
            lhs: ValueShape::Str,
            rhs: ValueShape::Array,
        }));
        assert_eq!(state, VerdictState::Fail);
        assert!(!check_is_retryable(state));

        // Missing data still reads as inconclusive (the environment may not
        // have produced the observation yet).
        assert_eq!(
            eval_to_state(&EvalResult::Inconclusive(EvalCause::MissingInput(
                "base_url".into()
            ))),
            VerdictState::Inconclusive(InconclusiveCause::MissingObservation),
        );
    }

    #[test]
    fn retry_classification_only_recoverable_inconclusive_retries() {
        // Fail and pass never retry; missing_observation /
        // empty_aggregation reflect the VD's shape, not flaky infra.
        assert!(check_is_retryable(VerdictState::Inconclusive(
            InconclusiveCause::Timeout
        )));
        assert!(check_is_retryable(VerdictState::Inconclusive(
            InconclusiveCause::EnvironmentError
        )));
        assert!(!check_is_retryable(VerdictState::Fail));
        assert!(!check_is_retryable(VerdictState::Pass));
        assert!(!check_is_retryable(VerdictState::Inconclusive(
            InconclusiveCause::MissingObservation
        )));
        assert!(!check_is_retryable(VerdictState::Inconclusive(
            InconclusiveCause::EmptyAggregation
        )));
    }

    #[test]
    fn retry_delay_scales_per_backoff() {
        let base = Duration::from_millis(100);
        // Linear: base · attempt.
        assert_eq!(retry_delay(base, RetryBackoff::Linear, 1), base);
        assert_eq!(
            retry_delay(base, RetryBackoff::Linear, 3),
            Duration::from_millis(300)
        );
        // Exponential: base · 2^(attempt-1).
        assert_eq!(retry_delay(base, RetryBackoff::Exponential, 1), base);
        assert_eq!(
            retry_delay(base, RetryBackoff::Exponential, 3),
            Duration::from_millis(400)
        );
    }

    #[test]
    fn default_within_fills_only_absent_within_on_a_mapping() {
        // Mapping without `within` → filled with the default (ms int).
        let mut empty = serde_yml::from_str::<serde_yml::Value>("{}").unwrap();
        apply_default_within(&mut empty, Duration::from_secs(7));
        assert_eq!(
            empty
                .get(serde_yml::Value::String("within".into()))
                .and_then(|v| v.as_u64()),
            Some(7_000)
        );
        // Mapping with its own `within` → untouched (per-step wins).
        let mut own = serde_yml::from_str::<serde_yml::Value>("within: 2000").unwrap();
        apply_default_within(&mut own, Duration::from_secs(7));
        assert_eq!(
            own.get(serde_yml::Value::String("within".into()))
                .and_then(|v| v.as_u64()),
            Some(2_000)
        );
        // Non-mapping (null `with:`) → left alone.
        let mut null = serde_yml::Value::Null;
        apply_default_within(&mut null, Duration::from_secs(7));
        assert!(null.is_null());
    }
}
