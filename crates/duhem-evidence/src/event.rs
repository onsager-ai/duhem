//! Event schema for the append-only run trace.
//!
//! Every line of `trace.jsonl` deserializes to exactly one [`Event`].
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

/// On-disk schema version carried in `manifest.json` and (redundantly)
/// in every `run_started` event. The redundancy is on purpose: the
/// manifest can be lost or copied without the directory and the trace
/// must still be self-describing.
pub const SCHEMA_VERSION: &str = "v1";

/// Inline-vs-blob threshold for `step_observation.value`. Values whose
/// serialized byte length exceeds this are written to `blobs/` and the
/// event carries `blob_sha256` instead.
pub const BLOB_INLINE_THRESHOLD_BYTES: usize = 4 * 1024;

/// Outcome of a single step invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Ok,
    Error,
    Timeout,
}

/// Outcome of evaluating a single assertion.
///
/// Three states by design — see `docs/duhem-spec.md` §7.6 / §11.2.
/// The judge's `aggregate_run` (spec landing in parallel) folds these
/// into a check / criterion / run verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionState {
    Pass,
    Fail,
    Inconclusive,
}

/// Aggregated verdict for a check, criterion, or run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
    Inconclusive,
}

/// Either an inline JSON value (small observations) or a reference to
/// a content-addressed blob (large observations). Exactly one variant
/// is serialized — `serde(untagged)` matches on the presence of the
/// `blob_sha256` key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ObservationValue {
    /// Blob reference. The bytes live at `blobs/<sha256>`.
    Blob { blob_sha256: String },
    /// Inline JSON value.
    Inline { value: serde_json::Value },
}

/// One line in `trace.jsonl`. The `seq` field is monotonic per run
/// (gap = bug) and `ts` is RFC 3339 with millisecond precision.
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
    StepStarted {
        criterion_id: String,
        check_id: String,
        step_index: u32,
        uses: String,
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
        state: AssertionState,
        #[serde(default)]
        detail: Option<String>,
    },
    CheckFinished {
        check_id: String,
        verdict: Verdict,
    },
    CriterionFinished {
        criterion_id: String,
        verdict: Verdict,
    },
    RunFinished {
        verdict: Verdict,
    },
}

impl EventPayload {
    /// Whether this payload requires an `fsync` after the line is
    /// written. The contract from issue #10: fsync at every
    /// `*_finished` event, buffer step observations.
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            EventPayload::StepFinished { .. }
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
    fn is_finished_flags_the_right_variants() {
        assert!(
            EventPayload::RunFinished {
                verdict: Verdict::Pass
            }
            .is_finished()
        );
        assert!(
            EventPayload::CheckFinished {
                check_id: "x".into(),
                verdict: Verdict::Pass
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
