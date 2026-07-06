//! `RunContext` — concrete `EvalContext` for an in-flight check.
//!
//! Two layers:
//!
//! - [`RunState`] is per-run, owned by the engine's `run()` frame. It
//!   carries the declared inputs (resolved from CLI args, typed per
//!   the input catalog), the whitelisted env, and the run's
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
        Self::new_inner(inputs, uuid::Uuid::new_v4().to_string())
    }

    /// Like [`RunState::new`] but derives `uuid` deterministically from
    /// a u64 seed instead of `Uuid::new_v4()`. Two runs with the same
    /// seed see the same `$runtime.uuid()` value (spec on issue #33).
    /// Scope is the cached `uuid` only — run IDs and event timestamps
    /// remain nondeterministic, so the event stream is not byte-identical
    /// across runs. The guarantee is over evaluator-visible entropy.
    /// The mapping is a splitmix64 expansion of the seed over 16 bytes
    /// followed by `Uuid::from_bytes`; collision resistance is not the
    /// property we're after, determinism is.
    pub fn new_with_seed(inputs: BTreeMap<String, Value>, seed: u64) -> Self {
        Self::new_inner(inputs, seeded_uuid(seed))
    }

    fn new_inner(inputs: BTreeMap<String, Value>, uuid: String) -> Self {
        Self {
            inputs,
            env: BTreeMap::new(),
            uuid,
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

/// Derive a deterministic uuid string from a u64 seed via splitmix64.
/// Used by [`RunState::new_with_seed`] to produce a stable
/// `$runtime.uuid()` value when the CLI passes `--seed`. Same seed →
/// same uuid, byte for byte.
fn seeded_uuid(seed: u64) -> String {
    let mut state = seed;
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        chunk.copy_from_slice(&z.to_le_bytes());
    }
    uuid::Uuid::from_bytes(bytes).to_string()
}

/// Total conversion from a JSON value (an action's output shape, or
/// a declared/coerced input) into the runtime `Value` model. Numerics
/// that fall outside `i64`/`f64` representable range return `None`;
/// every other shape is faithfully preserved, including arrays /
/// objects (typed-input catalog spec).
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
        J::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(json_to_value(item)?);
            }
            Value::Array(out)
        }
        J::Object(map) => {
            let mut out = BTreeMap::new();
            for (k, v) in map {
                out.insert(k.clone(), json_to_value(v)?);
            }
            Value::Object(out)
        }
    })
}

/// Inverse of [`json_to_value`] for substituting an evaluated value
/// back into a `serde_yml::Value` slot inside `Step.with`. Total —
/// every `Value` has a YAML representation.
pub fn value_to_yml(v: &Value) -> serde_yml::Value {
    match v {
        Value::Null => serde_yml::Value::Null,
        Value::Bool(b) => serde_yml::Value::Bool(*b),
        Value::Int(i) => serde_yml::to_value(*i).unwrap_or(serde_yml::Value::Null),
        Value::Float(f) => serde_yml::to_value(*f).unwrap_or(serde_yml::Value::Null),
        Value::Str(s) => serde_yml::Value::String(s.clone()),
        Value::Array(items) => serde_yml::Value::Sequence(items.iter().map(value_to_yml).collect()),
        Value::Object(map) => {
            let mut m = serde_yml::Mapping::new();
            for (k, v) in map {
                m.insert(serde_yml::Value::String(k.clone()), value_to_yml(v));
            }
            serde_yml::Value::Mapping(m)
        }
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
    fn env_whitelist_resolves_after_seeding() {
        // Spec #68: a selected environment's string-valued keys seed
        // the `$env.<key>` whitelist. With the map populated,
        // `EvalContext::env` resolves; an unseeded key stays `None`.
        let mut env = BTreeMap::new();
        env.insert("base_url".to_string(), "https://staging".to_string());
        let run = RunState::new(BTreeMap::new()).with_env(env);
        let ctx = RunContext::new(&run);
        assert_eq!(ctx.env("base_url"), Some("https://staging"));
        assert_eq!(ctx.env("missing"), None);
    }

    #[test]
    fn empty_env_whitelist_resolves_nothing() {
        // Regression: without a selected environment the whitelist is
        // empty and `$env.<key>` resolves to nothing (today's default).
        let run = RunState::new(BTreeMap::new());
        let ctx = RunContext::new(&run);
        assert_eq!(ctx.env("base_url"), None);
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
    fn seeded_uuid_is_deterministic_across_runstates() {
        // Spec on #33: byte-identical `$runtime.uuid()` for the same
        // seed. Two RunStates constructed with seed=42 must produce
        // exactly the same uuid string.
        let a = RunState::new_with_seed(BTreeMap::new(), 42);
        let b = RunState::new_with_seed(BTreeMap::new(), 42);
        assert_eq!(a.uuid, b.uuid);
        // Sanity: a different seed must produce a different uuid (the
        // splitmix64 expansion is injective on 64 bits).
        let c = RunState::new_with_seed(BTreeMap::new(), 43);
        assert_ne!(a.uuid, c.uuid);
        // Sanity: result is a parseable uuid.
        uuid::Uuid::parse_str(&a.uuid).expect("parseable uuid");
    }

    #[test]
    fn unseeded_runs_have_distinct_uuids() {
        // Regression: omitting `--seed` must preserve today's
        // nondeterministic uuid behavior.
        let a = RunState::new(BTreeMap::new());
        let b = RunState::new(BTreeMap::new());
        assert_ne!(a.uuid, b.uuid);
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
    }

    #[test]
    fn json_arrays_and_objects_convert_recursively() {
        // Typed-input catalog: declared `array` / `object` inputs flow
        // end-to-end through `json_to_value` rather than being dropped.
        let arr = json_to_value(&serde_json::json!([1, "two", true])).unwrap();
        assert_eq!(
            arr,
            Value::Array(vec![
                Value::Int(1),
                Value::Str("two".into()),
                Value::Bool(true),
            ])
        );
        let obj = json_to_value(&serde_json::json!({"k": 1, "nested": {"x": "y"}})).unwrap();
        let mut nested = BTreeMap::new();
        nested.insert("x".into(), Value::Str("y".into()));
        let mut top = BTreeMap::new();
        top.insert("k".into(), Value::Int(1));
        top.insert("nested".into(), Value::Object(nested));
        assert_eq!(obj, Value::Object(top));
    }
}
