//! `RunContext` — concrete `EvalContext` for an in-flight check.
//!
//! Two layers:
//!
//! - [`RunState`] is per-run, owned by the engine's `run()` frame. It
//!   carries the declared inputs (resolved from CLI args, strings
//!   only at v1), the whitelisted env, and the run's
//!   `$runtime.uuid()` value (computed once at run start so a
//!   definition that uses `$runtime.uuid()` twice in the same run
//!   sees the same value — the `"test-ws-{{uuid}}"` author-intent
//!   pattern from `docs/duhem-spec.md` §10.3).
//! - [`RunContext`] is per-check. It borrows `RunState` and owns its
//!   own map of observed step outputs. Every check view of a run
//!   sees the same inputs, env, and `uuid()` — only step outputs
//!   reset across checks.
//!
//! The spec on issue #15 calls this "uuid cache on the Engine"; the
//! implementation lives on `RunState` (per-run, constructed inside
//! `Engine::run`). Same end-to-end behavior; lifetime is the run, not
//! the engine handle.
//!
//! `$runtime.now()` is sampled fresh per call by the evaluator
//! (cached only over a single comparison — see `eval.rs`).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::eval::{EvalContext, Value};

/// Per-run state that survives across checks. Inputs are immutable
/// once the run starts; step outputs are appended to as steps run.
/// `uuid` is computed once at run start (held on the Engine) and
/// stays stable through every check.
#[derive(Debug)]
pub struct RunState {
    pub inputs: BTreeMap<String, Value>,
    pub env: BTreeMap<String, String>,
    pub uuid: String,
    /// `$setup.<step_id>.outputs.<name>` lookup map. Populated once
    /// by `Engine::run` from the run-level `setup:` block (issue #20)
    /// before any criterion runs; read-only from inside a check.
    pub setup_outputs: BTreeMap<(String, String), Value>,
}

impl RunState {
    pub fn new(inputs: BTreeMap<String, Value>) -> Self {
        Self {
            inputs,
            env: BTreeMap::new(),
            uuid: uuid::Uuid::new_v4().to_string(),
            setup_outputs: BTreeMap::new(),
        }
    }

    /// Override the env whitelist. Empty by default — env-access from
    /// assertions is opt-in via this map at v1.
    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Record an observed `$setup.<step_id>.outputs.<name>` value.
    /// Called by `Engine::run` while walking `def.setup`.
    pub fn record_setup_output(&mut self, step_id: &str, name: &str, value: Value) {
        self.setup_outputs
            .insert((step_id.to_string(), name.to_string()), value);
    }
}

/// `EvalContext` view for a single check. Borrows the run-level state
/// (inputs, env, uuid cache) and owns its own per-check map of
/// observed step outputs.
pub struct RunContext<'r> {
    run: &'r RunState,
    outputs: BTreeMap<(String, String), Value>,
}

impl<'r> RunContext<'r> {
    pub fn new(run: &'r RunState) -> Self {
        Self {
            run,
            outputs: BTreeMap::new(),
        }
    }

    /// Record an observed `$steps.<step_id>.outputs.<name>` value.
    pub fn record_output(&mut self, step_id: &str, name: &str, value: Value) {
        self.outputs
            .insert((step_id.to_string(), name.to_string()), value);
    }
}

impl<'r> EvalContext for RunContext<'r> {
    fn input(&self, name: &str) -> Option<&Value> {
        self.run.inputs.get(name)
    }

    fn output(&self, step_id: &str, output: &str) -> Option<&Value> {
        self.outputs.get(&(step_id.to_string(), output.to_string()))
    }

    fn setup_output(&self, step_id: &str, output: &str) -> Option<&Value> {
        self.run
            .setup_outputs
            .get(&(step_id.to_string(), output.to_string()))
    }

    fn env(&self, name: &str) -> Option<&str> {
        self.run.env.get(name).map(String::as_str)
    }

    fn uuid(&self) -> &str {
        &self.run.uuid
    }

    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Best-effort conversion from a JSON value (an action's output
/// shape) into the v1 scalar `Value` model. Returns `None` for
/// composite shapes (object/array) — those are out of scope until
/// the value model grows in a follow-up spec.
pub fn json_to_value(v: &serde_json::Value) -> Option<Value> {
    use serde_json::Value as J;
    Some(match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                return None;
            }
        }
        J::String(s) => Value::Str(s.clone()),
        J::Array(_) | J::Object(_) => return None,
    })
}

/// Inverse of [`json_to_value`] for substituting an evaluated scalar
/// back into a `serde_yml::Value` slot inside `Step.with`. Always
/// total — every scalar `Value` has a YAML representation.
pub fn value_to_yml(v: &Value) -> serde_yml::Value {
    match v {
        Value::Null => serde_yml::Value::Null,
        Value::Bool(b) => serde_yml::Value::Bool(*b),
        Value::Int(i) => serde_yml::to_value(*i).unwrap_or(serde_yml::Value::Null),
        Value::Float(f) => serde_yml::to_value(*f).unwrap_or(serde_yml::Value::Null),
        Value::Str(s) => serde_yml::Value::String(s.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_lookup() {
        let mut inputs = BTreeMap::new();
        inputs.insert("x".into(), Value::Int(42));
        let run = RunState::new(inputs);
        let ctx = RunContext::new(&run);
        assert_eq!(ctx.input("x"), Some(&Value::Int(42)));
        assert_eq!(ctx.input("y"), None);
    }

    #[test]
    fn output_lookup_after_record() {
        let run = RunState::new(BTreeMap::new());
        let mut ctx = RunContext::new(&run);
        ctx.record_output("s1", "code", Value::Int(200));
        assert_eq!(ctx.output("s1", "code"), Some(&Value::Int(200)));
        assert_eq!(ctx.output("s1", "missing"), None);
    }

    #[test]
    fn uuid_is_stable_within_a_run_across_check_views() {
        let run = RunState::new(BTreeMap::new());
        let a = RunContext::new(&run);
        let b = RunContext::new(&run);
        assert_eq!(a.uuid(), b.uuid());
    }

    #[test]
    fn json_scalars_round_trip_to_value() {
        assert_eq!(
            json_to_value(&serde_json::json!(true)),
            Some(Value::Bool(true))
        );
        assert_eq!(json_to_value(&serde_json::json!(7)), Some(Value::Int(7)));
        assert_eq!(
            json_to_value(&serde_json::json!("hi")),
            Some(Value::Str("hi".into()))
        );
        assert_eq!(json_to_value(&serde_json::json!(null)), Some(Value::Null));
        assert!(json_to_value(&serde_json::json!({})).is_none());
        assert!(json_to_value(&serde_json::json!([1, 2])).is_none());
    }
}
