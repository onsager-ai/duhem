//! Event schema for the append-only run trace.
//!
//! Every wire-format event line (an `events.payload` row, an export
//! bundle line) deserializes to exactly one [`Event`].
//! The variants here are the closed set: an unknown `kind` on read is
//! a hard error (see `reader.rs`). New kinds in future minor versions
//! are additive — existing kinds are stable, per the v1 schema
//! commitment in issue #10.

use std::collections::BTreeMap;
use std::time::Duration;

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

/// Runtime evidence heartbeat cadence. The CLI terminal renderer uses
/// the same period so operator narration and stored liveness cannot
/// drift apart.
pub const HEARTBEAT_PERIOD: Duration = Duration::from_secs(10);

/// Three missed runtime heartbeats make an unterminated run orphaned.
/// This is read-time policy: no `orphaned` fact is persisted.
pub const ORPHAN_THRESHOLD: Duration = Duration::from_secs(30);

/// Run-level action phase. Setup is the historical/default phase, so
/// old setup traces remain byte-identical; teardown reuses the same
/// lifecycle event family with an explicit discriminator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepPhase {
    #[default]
    Setup,
    Teardown,
}

impl StepPhase {
    pub fn is_setup(&self) -> bool {
        *self == Self::Setup
    }
}

/// Runtime-owned lifecycle, independent of the judge's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Finished,
    Aborted,
    Orphaned,
}

/// How a nested run came to exist. Root runs carry no origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOrigin {
    Suite,
    Invocation,
}

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
    Skipped { reason: String },
}

/// Assertion and finished-verdict fields share the same three-state
/// judgment vocabulary (`docs/duhem-spec.md` §7.6). A step that did not
/// run has no assertion event; its absence is carried by
/// [`StepOutcome::Skipped`] instead.
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
    Blob {
        blob_sha256: String,
        /// Non-zero exact-substring replacements made while writing this
        /// text artifact (spec #346). Absent for binary artifacts,
        /// unmasked text, and evidence recorded before the field existed.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        mask_counts: BTreeMap<String, u64>,
    },
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

/// Authored reusable-flow boundary for one expanded action step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowOrigin {
    pub name: String,
    pub invocation: String,
    pub inner_index: u32,
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
        /// Recorded lineage (#348). Absent on root runs and traces
        /// written before lineage existed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
        /// Why this child exists. Root and pre-lineage runs omit it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<RunOrigin>,
        /// Snapshot of the Verification Definition source (raw YAML) as
        /// it was when this run was judged (spec #302). Makes a run
        /// self-describing — the criteria/check descriptions, step ids,
        /// and assertion rules travel *with* the evidence, so a reader
        /// (the dashboard, an export on a dumb host) can show *what was
        /// verified* without the source file. Additive and backward
        /// compatible: a run predating this field deserializes with
        /// `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        definition: Option<String>,
        /// Effective browser viewport for this run. Fixed headless runs
        /// record `{width,height}`; headed/window-tracked runs record an
        /// explicit JSON `null`. Absent only on older/page-free records.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        viewport: Option<serde_json::Value>,
    },
    /// Periodic proof that the runtime still owns this unterminated run.
    RunHeartbeat,
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
        #[serde(default, skip_serializing_if = "StepPhase::is_setup")]
        phase: StepPhase,
        step_count: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixture_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check_id: Option<String>,
    },
    SetupStepStarted {
        #[serde(default, skip_serializing_if = "StepPhase::is_setup")]
        phase: StepPhase,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixture_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check_id: Option<String>,
    },
    SetupStepObservation {
        #[serde(default, skip_serializing_if = "StepPhase::is_setup")]
        phase: StepPhase,
        step_index: u32,
        output_name: String,
        #[serde(flatten)]
        value: ObservationValue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixture_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check_id: Option<String>,
    },
    SetupStepFinished {
        #[serde(default, skip_serializing_if = "StepPhase::is_setup")]
        phase: StepPhase,
        step_index: u32,
        outcome: StepOutcome,
        /// Human-readable cause for an error or timeout, masked at the
        /// evidence boundary (#494). Absent for successful/skipped steps
        /// and traces recorded before this field existed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixture_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check_id: Option<String>,
    },
    SetupFinished {
        #[serde(default, skip_serializing_if = "StepPhase::is_setup")]
        phase: StepPhase,
        aborted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixture_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check_id: Option<String>,
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
        /// Reusable-flow origin for an expanded action. Absent on
        /// ordinary steps and on traces recorded before spec #367.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flow: Option<FlowOrigin>,
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
        /// Human-readable cause for an error or timeout, masked at the
        /// evidence boundary (#494). Absent for successful/skipped steps
        /// and traces recorded before this field existed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    AssertionEvaluated {
        check_id: String,
        assertion_index: u32,
        state: VerdictState,
        #[serde(default)]
        detail: Option<String>,
        /// The human-readable assertion *expression* this outcome
        /// evaluated — the explicit `assertions:` line as rendered by
        /// `Assertion::display` (e.g. `$steps.ok.outputs.exit_code ==
        /// 1`), or `step `<id>` satisfied == true` for an implicit
        /// judgment (spec #253). Recorded so a reporter can show *what
        /// was asserted*, not only the observed operands carried in
        /// `detail`. Additive and backward compatible: an event
        /// predating this field deserializes with `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expr: Option<String>,
        /// The step this assertion is derived from, when the link is
        /// known, so a reporter can fold the assertion into its step and
        /// propagate its status (a step whose assertion failed is a
        /// failed step, not a green one). Set for an *implicit* judgment
        /// (spec #253) — whose outcome IS a step's `satisfied` verdict —
        /// AND for an *explicit* `assertions:` entry that references
        /// exactly one `$steps.<id>` (e.g. `$steps.update.outputs.status
        /// == 200` folds onto the `update` step). `None` when the
        /// assertion references zero or many steps. Additive and
        /// backward compatible: an event predating this field
        /// deserializes with `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step_index: Option<u32>,
    },
    CheckFinished {
        check_id: String,
        verdict: VerdictState,
        /// Literal `session:` expression selected by the check. The
        /// credential value is never recorded; this source proves which
        /// declared baseline was requested (spec #347).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_source: Option<String>,
        /// Lowercase SHA-256 of the resolved storage-state JSON. Lets
        /// traces prove two isolated contexts began from the same
        /// baseline without carrying that baseline.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_digest: Option<String>,
    },
    CriterionFinished {
        criterion_id: String,
        verdict: VerdictState,
    },
    RunFinished {
        /// A run can complete without being judged. The suite-level
        /// environment container is the canonical example.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        verdict: Option<VerdictState>,
    },
    /// The runtime observed an operator termination signal and sealed
    /// the run before exiting.
    RunAborted {
        signal: String,
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
            EventPayload::RunHeartbeat => "run_heartbeat",
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
            EventPayload::RunAborted { .. } => "run_aborted",
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
                | EventPayload::RunAborted { .. }
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
                parent_run_id: None,
                origin: None,
                definition: None,
                viewport: None,
            },
        };
        let line = serde_json::to_string(&evt).unwrap();
        assert!(line.contains(r#""kind":"run_started""#));
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(evt, back);
    }

    #[test]
    fn run_started_distinguishes_fixed_window_and_legacy_viewports() {
        let fixed = crate::writer::run_started_with_viewport(
            "v.yml",
            BTreeMap::new(),
            None,
            crate::RunLineage::default(),
            Some(serde_json::json!({ "width": 1440, "height": 900 })),
        );
        let window = crate::writer::run_started_with_viewport(
            "v.yml",
            BTreeMap::new(),
            None,
            crate::RunLineage::default(),
            Some(serde_json::Value::Null),
        );
        let fixed_json = serde_json::to_value(fixed).unwrap();
        let window_json = serde_json::to_value(window).unwrap();
        assert_eq!(
            fixed_json["viewport"],
            serde_json::json!({ "width": 1440, "height": 900 })
        );
        assert!(window_json.get("viewport").is_some());
        assert!(window_json["viewport"].is_null());

        let legacy: EventPayload = serde_json::from_value(serde_json::json!({
            "kind": "run_started",
            "verification_path": "v.yml",
            "schema_version": "v1"
        }))
        .unwrap();
        assert!(matches!(
            legacy,
            EventPayload::RunStarted { viewport: None, .. }
        ));
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
                    mask_counts: BTreeMap::new(),
                },
            },
        };
        let line = serde_json::to_string(&blob).unwrap();
        assert!(line.contains(r#""blob_sha256":"abc123""#));
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(blob, back);
    }

    #[test]
    fn skipped_step_outcome_round_trips_with_reason() {
        let evt = Event {
            seq: 2,
            ts: ts(),
            payload: EventPayload::StepFinished {
                step_index: 1,
                outcome: StepOutcome::Skipped {
                    reason: "blocked by failed step `login`".into(),
                },
                detail: None,
            },
        };
        let line = serde_json::to_string(&evt).unwrap();
        assert!(line.contains(r#""skipped":{"reason":"blocked by failed step `login`"}"#));
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), evt);
    }

    #[test]
    fn pre_detail_step_finishes_keep_their_wire_shape() {
        // Additive detail must not make a legacy error look newly
        // explained, nor change its bytes when read and re-emitted (#494).
        for old in [
            r#"{"seq":2,"ts":"2026-05-08T12:00:00.000Z","kind":"step_finished","step_index":1,"outcome":"error"}"#,
            r#"{"seq":2,"ts":"2026-05-08T12:00:00.000Z","kind":"setup_step_finished","step_index":1,"outcome":"timeout"}"#,
        ] {
            let event: Event = serde_json::from_str(old).unwrap();
            match &event.payload {
                EventPayload::StepFinished { detail, .. }
                | EventPayload::SetupStepFinished { detail, .. } => {
                    assert_eq!(detail, &None);
                }
                other => panic!("unexpected payload: {other:?}"),
            }
            assert_eq!(serde_json::to_string(&event).unwrap(), old);
        }
    }

    #[test]
    fn unknown_kind_is_a_hard_error() {
        let bad = r#"{"seq":0,"ts":"2026-05-08T12:00:00.000Z","kind":"made_up"}"#;
        assert!(serde_json::from_str::<Event>(bad).is_err());
    }

    #[test]
    fn pre_lifecycle_run_finished_keeps_its_wire_shape() {
        let old =
            r#"{"seq":7,"ts":"2026-05-08T12:00:00.000Z","kind":"run_finished","verdict":"pass"}"#;
        let event: Event = serde_json::from_str(old).unwrap();
        assert_eq!(
            event.payload,
            EventPayload::RunFinished {
                verdict: Some(VerdictState::Pass)
            }
        );
        assert_eq!(serde_json::to_string(&event).unwrap(), old);
    }

    #[test]
    fn check_session_metadata_is_additive_and_credential_free() {
        let old = r#"{"seq":7,"ts":"2026-05-08T12:00:00.000Z","kind":"check_finished","check_id":"AC-1.1","verdict":"pass"}"#;
        let event: Event = serde_json::from_str(old).unwrap();
        assert_eq!(serde_json::to_string(&event).unwrap(), old);

        let seeded = Event {
            seq: 8,
            ts: ts(),
            payload: EventPayload::CheckFinished {
                check_id: "AC-2.1".into(),
                verdict: VerdictState::Pass,
                session_source: Some("$setup.session.outputs.state".into()),
                session_digest: Some("a".repeat(64)),
            },
        };
        let line = serde_json::to_string(&seeded).unwrap();
        assert!(line.contains(r#""session_source":"$setup.session.outputs.state""#));
        assert!(line.contains(&format!(r#""session_digest":"{}""#, "a".repeat(64))));
        assert!(!line.contains("cookies"));
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), seeded);
    }

    #[test]
    fn setup_variants_round_trip() {
        let cases: Vec<EventPayload> = vec![
            EventPayload::SetupStarted {
                phase: StepPhase::Setup,
                step_count: 2,
                fixture_name: None,
                check_id: None,
            },
            EventPayload::SetupStepStarted {
                phase: StepPhase::Setup,
                step_index: 0,
                uses: "ui/navigate".into(),
                layer: None,
                with: BTreeMap::new(),
                fixture_name: None,
                check_id: None,
            },
            EventPayload::SetupStepObservation {
                phase: StepPhase::Setup,
                step_index: 0,
                output_name: "landed_at".into(),
                value: ObservationValue::Inline {
                    value: serde_json::json!("http://x/"),
                },
                fixture_name: None,
                check_id: None,
            },
            EventPayload::SetupStepFinished {
                phase: StepPhase::Setup,
                step_index: 0,
                outcome: StepOutcome::Ok,
                detail: None,
                fixture_name: None,
                check_id: None,
            },
            EventPayload::SetupFinished {
                phase: StepPhase::Setup,
                aborted: false,
                fixture_name: None,
                check_id: None,
            },
        ];
        for payload in cases {
            let evt = Event {
                seq: 1,
                ts: ts(),
                payload,
            };
            let line = serde_json::to_string(&evt).unwrap();
            assert!(!line.contains("\"phase\""), "setup wire drifted: {line}");
            let back: Event = serde_json::from_str(&line).unwrap();
            assert_eq!(evt, back, "round-trip via {line}");
        }
    }

    #[test]
    fn teardown_reuses_setup_step_events_with_a_phase_discriminator() {
        let event = Event {
            seq: 1,
            ts: ts(),
            payload: EventPayload::SetupStepFinished {
                phase: StepPhase::Teardown,
                step_index: 0,
                outcome: StepOutcome::Error,
                detail: None,
                fixture_name: None,
                check_id: None,
            },
        };
        let line = serde_json::to_string(&event).unwrap();
        assert!(line.contains(r#""phase":"teardown""#), "got: {line}");
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), event);
    }

    #[test]
    fn setup_finished_is_a_finished_event() {
        // Setup spec on #20: `SetupFinished` fsyncs (same rule as the
        // other `*_finished` events in #10). `is_finished()` is the
        // wire on that policy.
        assert!(
            EventPayload::SetupFinished {
                phase: StepPhase::Setup,
                aborted: false,
                fixture_name: None,
                check_id: None
            }
            .is_finished()
        );
        assert!(
            EventPayload::SetupFinished {
                phase: StepPhase::Setup,
                aborted: true,
                fixture_name: None,
                check_id: None
            }
            .is_finished()
        );
        assert!(
            EventPayload::SetupStepFinished {
                phase: StepPhase::Setup,
                step_index: 0,
                outcome: StepOutcome::Ok,
                detail: None,
                fixture_name: None,
                check_id: None,
            }
            .is_finished()
        );
        // Setup-side started / observation events are non-finishing,
        // same as their per-check counterparts.
        assert!(
            !EventPayload::SetupStarted {
                phase: StepPhase::Setup,
                step_count: 1,
                fixture_name: None,
                check_id: None
            }
            .is_finished()
        );
        assert!(
            !EventPayload::SetupStepObservation {
                phase: StepPhase::Setup,
                step_index: 0,
                output_name: "n".into(),
                value: ObservationValue::Inline {
                    value: serde_json::json!(1),
                },
                fixture_name: None,
                check_id: None,
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
                verdict: Some(VerdictState::Pass)
            }
            .is_finished()
        );
        assert!(
            EventPayload::CheckFinished {
                check_id: "x".into(),
                verdict: VerdictState::Pass,
                session_source: None,
                session_digest: None,
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

    #[test]
    fn step_started_flow_origin_is_optional_and_round_trips() {
        let old = r#"{"kind":"step_started","criterion_id":"AC-1","check_id":"AC-1.1","step_index":0,"uses":"ui/click"}"#;
        let parsed: EventPayload = serde_json::from_str(old).expect("old trace parses");
        assert!(matches!(
            parsed,
            EventPayload::StepStarted { flow: None, .. }
        ));

        let payload = EventPayload::StepStarted {
            criterion_id: "AC-1".into(),
            check_id: "AC-1.1".into(),
            step_index: 0,
            uses: "ui/click".into(),
            layer: Some("ui".into()),
            with: BTreeMap::new(),
            flow: Some(FlowOrigin {
                name: "sign_in".into(),
                invocation: "login".into(),
                inner_index: 2,
            }),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert_eq!(
            serde_json::from_str::<EventPayload>(&json).expect("round trip"),
            payload
        );
    }
}
