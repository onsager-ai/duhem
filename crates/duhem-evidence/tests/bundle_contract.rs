//! The run-bundle wire contract (#194).
//!
//! The closed-source hub builds against this shape (#188 open-core
//! seam), so the envelope is pinned here: an intentional change must
//! update the golden below AND bump `BUNDLE_VERSION`, and the PR
//! carries the cross-repo coordination. An accidental drift fails
//! this test.

use std::collections::BTreeMap;
use std::sync::Arc;

use duhem_evidence::{
    BUNDLE_VERSION, BundleArtifact, BundleRun, Event, EventPayload, EvidenceWriter,
    ObservationValue, RunBundle, SESSION_EVIDENCE_OBSERVATION, STEP_SCREENSHOT_OBSERVATION,
    SessionEvidence, SessionStep, SqliteStore, VerdictState, run_started,
};

fn fixed_bundle() -> RunBundle {
    RunBundle {
        bundle_version: BUNDLE_VERSION,
        run: BundleRun {
            run_id: "01RUN".into(),
            verification: "verifications/login/duhem.yml".into(),
            schema_version: "v1".into(),
            inputs: BTreeMap::from([("base_url".to_string(), serde_json::json!("http://sut"))]),
            started_at: "2026-07-06T00:00:00.000Z".into(),
            verdict: Some(VerdictState::Pass),
            finished_at: Some("2026-07-06T00:00:01.000Z".into()),
            duration_ms: Some(1000),
            project_id: Some("github.com/acme/app".into()),
            verifier_repo: Some("github.com/onsager-ai/duhem".into()),
            verifier_sha: Some("deadbeef".into()),
            target_repo: Some("github.com/acme/app".into()),
            target_sha: Some("cafef00d".into()),
        },
        events: vec![Event {
            seq: 0,
            ts: "2026-07-06T00:00:00.000Z".parse().unwrap(),
            payload: EventPayload::RunFinished {
                verdict: Some(VerdictState::Pass),
            },
        }],
        artifacts: vec![BundleArtifact {
            sha256: "aa".repeat(32),
            bytes_base64: "aGVsbG8=".into(),
            mask_counts: BTreeMap::new(),
        }],
    }
}

/// The golden envelope. If this assertion fails you changed the wire
/// contract: bump `BUNDLE_VERSION`, update this golden, and
/// coordinate the hub-side change (#188 sibling repo).
#[test]
fn wire_envelope_shape_is_pinned() {
    let wire = String::from_utf8(fixed_bundle().wire_bytes().unwrap()).unwrap();
    let golden = concat!(
        r#"{"bundle_version":1,"run":{"run_id":"01RUN","#,
        r#""verification":"verifications/login/duhem.yml","schema_version":"v1","#,
        r#""inputs":{"base_url":"http://sut"},"started_at":"2026-07-06T00:00:00.000Z","#,
        r#""verdict":"pass","finished_at":"2026-07-06T00:00:01.000Z","duration_ms":1000,"#,
        r#""project_id":"github.com/acme/app","verifier_repo":"github.com/onsager-ai/duhem","#,
        r#""verifier_sha":"deadbeef","target_repo":"github.com/acme/app","target_sha":"cafef00d"},"#,
        r#""events":[{"seq":0,"ts":"2026-07-06T00:00:00.000Z","kind":"run_finished","verdict":"pass"}],"#,
        r#""artifacts":[{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","bytes_base64":"aGVsbG8="}]}"#,
    );
    assert_eq!(wire, golden, "bundle wire shape drifted — see test doc");
}

#[test]
fn content_hash_is_deterministic_and_shape_sensitive() {
    let a = fixed_bundle();
    let b = fixed_bundle();
    assert_eq!(a.content_hash().unwrap(), b.content_hash().unwrap());
    let mut c = fixed_bundle();
    c.run.target_sha = Some("other".into());
    assert_ne!(a.content_hash().unwrap(), c.content_hash().unwrap());
}

#[tokio::test]
async fn store_to_bundle_round_trips_through_the_export_directory() {
    // A real run through the writer, bundled, written as the export
    // dir, read back — envelope and hash identical (export and ship
    // are one artifact, two destinations).
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .unwrap(),
    );
    let mut w = EvidenceWriter::begin(store.clone(), "01RT", "x.yml", BTreeMap::new())
        .await
        .unwrap();
    w.append(run_started("x.yml", BTreeMap::new()))
        .await
        .unwrap();
    // A blob observation so artifacts are exercised.
    let big = "z".repeat(5 * 1024);
    w.append_observation(0, "payload", serde_json::json!(big))
        .await
        .unwrap();
    // A referenced storyboard PNG plus the versioned session document.
    // Both must survive the same bundle round-trip as ordinary blobs.
    let shot = w.write_blob(b"\x89PNG\r\n\x1a\nframe").await.unwrap();
    w.append(EventPayload::StepObservation {
        step_index: 0,
        output_name: STEP_SCREENSHOT_OBSERVATION.into(),
        value: ObservationValue::Blob {
            blob_sha256: shot.0.clone(),
            mask_counts: BTreeMap::new(),
        },
    })
    .await
    .unwrap();
    let mut session = SessionEvidence::v1(10.0);
    session.steps.push(SessionStep {
        step_index: 0,
        started_ms: 1.0,
        finished_ms: 9.0,
        screenshot_sha256: Some(shot.0),
        screenshot_ms: Some(9.0),
        frame_error: None,
    });
    let session_blob = w
        .write_blob(&serde_json::to_vec(&session).unwrap())
        .await
        .unwrap();
    w.append(EventPayload::StepObservation {
        step_index: 0,
        output_name: SESSION_EVIDENCE_OBSERVATION.into(),
        value: ObservationValue::Blob {
            blob_sha256: session_blob.0.clone(),
            mask_counts: BTreeMap::new(),
        },
    })
    .await
    .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: Some(VerdictState::Pass),
    })
    .await
    .unwrap();

    let bundle = RunBundle::from_store(store.as_ref(), "01RT").await.unwrap();
    assert_eq!(bundle.artifacts.len(), 3);

    let out = tmp.path().join("export");
    bundle.write_dir(&out).unwrap();
    assert!(out.join("bundle.json").is_file());
    assert!(out.join("events.jsonl").is_file());

    let back = RunBundle::from_dir(&out).unwrap();
    assert_eq!(back, bundle);
    assert_eq!(back.content_hash().unwrap(), bundle.content_hash().unwrap());

    // Idempotency root: bundling the same immutable run twice yields
    // the same content hash.
    let again = RunBundle::from_store(store.as_ref(), "01RT").await.unwrap();
    assert_eq!(
        again.content_hash().unwrap(),
        bundle.content_hash().unwrap()
    );
    let encoded = back
        .artifacts
        .iter()
        .find(|artifact| artifact.sha256 == session_blob.0)
        .unwrap();
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&encoded.bytes_base64)
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<SessionEvidence>(&bytes).unwrap(),
        session
    );
}
