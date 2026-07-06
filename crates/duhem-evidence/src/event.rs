//! Event schema for the append-only run trace.
//!
//! Every wire-format event line (an `events.payload` row, an export
//! bundle line) deserializes to exactly one [`Event`].
//! The variants here are the closed set: an unknown `kind` on read is
//! a hard error (see `reader.rs`). New kinds in future minor versions
//! are additive — existing kinds are stable, per the v1 schema
//! commitment in issue #10.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Custom serde for `DateTime<Utc>` that always emits RFC 3339 with
/// exactly millisecond precision (`...:SS.sssZ`). The spec pins the
/// wire format at ms; in-memory values may carry more precision but
/// the on-disk representation must not.
pub(crate) mod ts_ms {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let s = String::deserialize(d)?;
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    }
}

/// Trace wire version carried on the run header row and (redundantly)
/// in every `run_started` event. The redundancy is on purpose: an
/// exported event stream must stay self-describing without the store
/// row next to it.
pub const SCHEMA_VERSION: &str = "v1";

/// Inline-vs-blob threshold for `step_observation.value`. Values whose
/// serialized byte length exceeds this are written to the artifact
/// store and the event carries `blob_sha256` instead.
pub const BLOB_INLINE_THRESHOLD_BYTES: usize = 4 * 1024;

/// Outcome of a single step invocation. Distinct from a verdict —
/// this answers "did the action complete?", not "did the artifact
/// pass?". A step can finish `ok` yet feed an `assertion_evaluated`
/// with `state: fail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Ok,
    Error,
    Timeout,
}

/// Verdict shape for `assertion_evaluated.state` and the three
/// `*_finished.verdict` fields. Re-exported from `duhem-judge` so the
/// evidence wire format and the judge's output share one canonical
/// type — the wire token is `"pass"` / `"fail"` /
/// `"inconclusive:<cause>"` (`docs/duhem-spec.md` §7.6).
pub use duhem_judge::VerdictState;

/// Either an inline JSON value (small observations) or a reference to
/// a content-addressed blob (large observations). Exactly one variant
/// is serialized — `serde(untagged)` matches on the presence of the
/// `blob_sha256` key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ObservationValue {
    /// Blob reference. The bytes live in the artifact store under
    /// this content address.
    Blob { blob_sha256: String },
    /// Inline JSON value.
    Inline { value: serde_json::Value },
}

/// One event on a run's wire-format stream. The `seq` field is
/// monotonic per run (gap = bug) and `ts` is RFC 3339 with
/// millisecond precision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic per run, starting at 0. A backwards-or-flat seq on
    /// read is a hard error.
    pub seq: u64,

    /// Wall-clock timestamp, RFC 3339, millisecond precision.
    #[serde(with = "ts_ms")]
    pub ts: DateTime<Utc>,

    /// Payload variant — the `kind` tag selects which fields are
    /// populated.
    #[serde(flatten)]
    pub payload: EventPayload,
}

/// The closed set of event payloads. `#[serde(tag = "kind")]` puts the
/// discriminant alongside `seq` and `ts` on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventPayload {
    RunStarted {
        verification_path: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        inputs: BTreeMap<String, serde_json::Value>,
        schema_version: String,
    },
    EnvUpStarted {
        command: String,
    },
    EnvUpFinished {
        exit_code: i32,
        duration_ms: u64,
        /// Content-addressed artifact reference for the captured
        /// stdout stream. `None` when the script produced no stdout
        /// (or `--no-env-up` skipped the invocation).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stdout_blob_sha256: Option<String>,
        /// Same shape as `stdout_blob_sha256` for stderr.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stderr_blob_sha256: Option<String>,
    },
    EnvReady {
        /// Identifier of the probe kind that ran. v1 emits `"http"`;
        /// future probe kinds widen the catalog without renaming this
        /// field.
        probe_kind: String,
        /// `true` when the probe observed the readiness signal within
        /// the configured timeout; `false` on timeout.
        ok: bool,
        elapsed_ms: u64,
    },
    EnvDownStarted {
        command: String,
    },
    EnvDownFinished {
        exit_code: i32,
        duration_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stdout_blob_sha256: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stderr_blob_sha256: Option<String>,
    },
    SetupStarted {
        step_count: u32,
    },
    SetupStepStarted {
        step_index: u32,
        uses: String,
        /// Delivery-web layer the executed action exercised (#192):
        /// `ui` / `api` / `data` / `runtime`. Stamped by the runtime
        /// from the action catalog family — never inferred. Absent
        /// for pre-tag traces and for `uses` outside the catalog
        /// families (untagged, not guessed). Additive to the #10
        /// wire shape.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layer: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        with: BTreeMap<String, serde_json::Value>,
    },
    SetupStepObservation {
        step_index: u32,
        output_name: String,
        #[serde(flatten)]
        value: ObservationValue,
    },
    SetupStepFinished {
        step_index: u32,
        outcome: StepOutcome,
    },
    SetupFinished {
        aborted: bool,
    },
    StepStarted {
        criterion_id: String,
        check_id: String,
        step_index: u32,
        uses: String,
        /// Delivery-web layer the executed action exercised (#192).
        /// Same contract as `SetupStepStarted.layer`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layer: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        with: BTreeMap<String, serde_json::Value>,
    },
    StepObservation {
        step_index: u32,
        output_name: String,
        #[serde(flatten)]
        value: ObservationValue,
    },
    StepFinished {
        step_index: u32,
        outcome: StepOutcome,
    },
    AssertionEvaluated {
        check_id: String,
        assertion_index: u32,
        state: VerdictState,
        #[serde(default)]
        detail: Option<String>,
    },
    CheckFinished {
        check_id: String,
        verdict: VerdictState,
    },
    CriterionFinished {
        criterion_id: String,
        verdict: VerdictState,
    },
    RunFinished {
        verdict: VerdictState,
    },
}

impl EventPayload {
    /// The wire discriminant for this payload — the same string the
    /// `kind` tag carries on the serialized form. The store keeps it
    /// in a dedicated column so events can be filtered without JSON
    /// extraction.
    pub fn kind(&self) -> &'static str {
        match self {
            EventPayload::RunStarted { .. } => "run_started",
            EventPayload::EnvUpStarted { .. } => "env_up_started",
            EventPayload::EnvUpFinished { .. } => "env_up_finished",
            EventPayload::EnvReady { .. } => "env_ready",
            EventPayload::EnvDownStarted { .. } => "env_down_started",
            EventPayload::EnvDownFinished { .. } => "env_down_finished",
            EventPayload::SetupStarted { .. } => "setup_started",
            EventPayload::SetupStepStarted { .. } => "setup_step_started",
            EventPayload::SetupStepObservation { .. } => "setup_step_observation",
            EventPayload::SetupStepFinished { .. } => "setup_step_finished",
            EventPayload::SetupFinished { .. } => "setup_finished",
            EventPayload::StepStarted { .. } => "step_started",
            EventPayload::StepObservation { .. } => "step_observation",
            EventPayload::StepFinished { .. } => "step_finished",
            EventPayload::AssertionEvaluated { .. } => "assertion_evaluated",
            EventPayload::CheckFinished { .. } => "check_finished",
            EventPayload::CriterionFinished { .. } => "criterion_finished",
            EventPayload::RunFinished { .. } => "run_finished",
        }
    }

    /// Whether this payload requires an `fsync` after the line is
    /// written. The contract from issue #10: fsync at every
    /// `*_finished` event, buffer step observations.
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            EventPayload::EnvUpFinished { .. }
                | EventPayload::EnvDownFinished { .. }
                | EventPayload::SetupStepFinished { .. }
                | EventPayload::SetupFinished { .. }
                | EventPayload::StepFinished { .. }
                | EventPayload::CheckFinished { .. }
                | EventPayload::CriterionFinished { .. }
                | EventPayload::RunFinished { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> DateTime<Utc> {
        "2026-05-08T12:00:00.000Z".parse().unwrap()
    }

    #[test]
    fn run_started_roundtrip() {
        let mut inputs = BTreeMap::new();
        inputs.insert("workspace_name".into(), serde_json::json!("test-ws-018f"));
        let evt = Event {
            seq: 0,
            ts: ts(),
            payload: EventPayload::RunStarted {
                verification_path: "create-workspace.yml".into(),
                inputs,
                schema_version: SCHEMA_VERSION.into(),
            },
        };
        let line = serde_json::to_string(&evt).unwrap();
        assert!(line.contains(r#""kind":"run_started""#));
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(evt, back);
    }

    #[test]
    fn step_observation_inline_vs_blob() {
        let inline = Event {
            seq: 1,
            ts: ts(),
            payload: EventPayload::StepObservation {
                step_index: 0,
                output_name: "count".into(),
                value: ObservationValue::Inline {
                    value: serde_json::json!(3),
                },
            },
        };
        let line = serde_json::to_string(&inline).unwrap();
        assert!(line.contains(r#""value":3"#));
        assert!(!line.contains("blob_sha256"));

        let blob = Event {
            seq: 2,
            ts: ts(),
            payload: EventPayload::StepObservation {
                step_index: 0,
                output_name: "screenshot".into(),
                value: ObservationValue::Blob {
                    blob_sha256: "abc123".into(),
                },
            },
        };
        let line = serde_json::to_string(&blob).unwrap();
        assert!(line.contains(r#""blob_sha256":"abc123""#));
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(blob, back);
    }

    #[test]
    fn unknown_kind_is_a_hard_error() {
        let bad = r#"{"seq":0,"ts":"2026-05-08T12:00:00.000Z","kind":"made_up"}"#;
        assert!(serde_json::from_str::<Event>(bad).is_err());
    }

    #[test]
    fn setup_variants_round_trip() {
        let cases: Vec<EventPayload> = vec![
            EventPayload::SetupStarted { step_count: 2 },
            EventPayload::SetupStepStarted {
                step_index: 0,
                uses: "ui/navigate".into(),
                layer: None,
                with: BTreeMap::new(),
            },
            EventPayload::SetupStepObservation {
                step_index: 0,
                output_name: "landed_at".into(),
                value: ObservationValue::Inline {
                    value: serde_json::json!("http://x/"),
                },
            },
            EventPayload::SetupStepFinished {
                step_index: 0,
                outcome: StepOutcome::Ok,
            },
            EventPayload::SetupFinished { aborted: false },
        ];
        for payload in cases {
            let evt = Event {
                seq: 1,
                ts: ts(),
                payload,
            };
            let line = serde_json::to_string(&evt).unwrap();
            let back: Event = serde_json::from_str(&line).unwrap();
            assert_eq!(evt, back, "round-trip via {line}");
        }
    }

    #[test]
    fn setup_finished_is_a_finished_event() {
        // Setup spec on #20: `SetupFinished` fsyncs (same rule as the
        // other `*_finished` events in #10). `is_finished()` is the
        // wire on that policy.
        assert!(EventPayload::SetupFinished { aborted: false }.is_finished());
        assert!(EventPayload::SetupFinished { aborted: true }.is_finished());
        assert!(
            EventPayload::SetupStepFinished {
                step_index: 0,
                outcome: StepOutcome::Ok,
            }
            .is_finished()
        );
        // Setup-side started / observation events are non-finishing,
        // same as their per-check counterparts.
        assert!(!EventPayload::SetupStarted { step_count: 1 }.is_finished());
        assert!(
            !EventPayload::SetupStepObservation {
                step_index: 0,
                output_name: "n".into(),
                value: ObservationValue::Inline {
                    value: serde_json::json!(1),
                },
            }
            .is_finished()
        );
    }

    #[test]
    fn env_variants_round_trip_and_finished_variants_are_flagged() {
        let cases: Vec<EventPayload> = vec![
            EventPayload::EnvUpStarted {
                command: "./scripts/up.sh".into(),
            },
            EventPayload::EnvUpFinished {
                exit_code: 0,
                duration_ms: 1234,
                stdout_blob_sha256: Some("a".repeat(64)),
                stderr_blob_sha256: None,
            },
            EventPayload::EnvReady {
                probe_kind: "http".into(),
                ok: true,
                elapsed_ms: 250,
            },
            EventPayload::EnvDownStarted {
                command: "./scripts/down.sh".into(),
            },
            EventPayload::EnvDownFinished {
                exit_code: 0,
                duration_ms: 50,
                stdout_blob_sha256: None,
                stderr_blob_sha256: None,
            },
        ];
        for payload in cases {
            let evt = Event {
                seq: 1,
                ts: ts(),
                payload: payload.clone(),
            };
            let line = serde_json::to_string(&evt).unwrap();
            let back: Event = serde_json::from_str(&line).unwrap();
            assert_eq!(evt, back, "round-trip via {line}");
            // `EnvUpFinished` / `EnvDownFinished` are *_finished; the
            // started / ready variants are non-finishing.
            let started = matches!(
                payload,
                EventPayload::EnvUpStarted { .. }
                    | EventPayload::EnvDownStarted { .. }
                    | EventPayload::EnvReady { .. }
            );
            assert_eq!(payload.is_finished(), !started, "for {line}");
        }
    }

    #[test]
    fn is_finished_flags_the_right_variants() {
        assert!(
            EventPayload::RunFinished {
                verdict: VerdictState::Pass
            }
            .is_finished()
        );
        assert!(
            EventPayload::CheckFinished {
                check_id: "x".into(),
                verdict: VerdictState::Pass
            }
            .is_finished()
        );
        assert!(
            !EventPayload::StepObservation {
                step_index: 0,
                output_name: "n".into(),
                value: ObservationValue::Inline {
                    value: serde_json::json!(1)
                }
            }
            .is_finished()
        );
    }
}
