//! `GET /api/runs/:id/failure` (#216): the agent failure envelope.
//! Seeds a failing run with layer-tagged spans + capture artifacts +
//! a network capture carrying a 500, and asserts the envelope hands an
//! agent the failing assertion, layer chain, artifact URLs, and the
//! first failing request — everything to react without scraping the UI.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use duhem_dashboard::{EvidenceReader, router};
use duhem_evidence::{
    EventPayload, EvidenceWriter, ObservationValue, RunLineage, RunOrigin, RunScope, SqliteStore,
    StepOutcome, VerdictState, run_started_with_definition_and_lineage,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const RUN: &str = "01FAILENVELOPE00000000000A";
const PASS: &str = "01FAILENVELOPE00000000000B";
const DEF: &str = "verifications/checkout.yml";

async fn get_json(reader: EvidenceReader, path: &str) -> (StatusCode, Value) {
    let res = router(reader)
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn step_started(idx: u32, uses: &str) -> EventPayload {
    EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: idx,
        uses: uses.into(),
        layer: Some("ui".into()),
        flow: None,
        with: BTreeMap::new(),
        group: None,
    }
}

fn good_har() -> Value {
    json!({ "log": { "entries": [
        { "request": { "method": "GET", "url": "http://x/" }, "response": { "status": 200 } },
        { "request": { "method": "POST", "url": "http://x/api/charge" }, "response": { "status": 500 } },
    ] } })
}

/// A finished run of one criterion / one check. `pass` drives the
/// verdict + assertion; a failing run records a screenshot and, when
/// `network` is given, a `capture/network` HAR blob.
async fn seed(store: Arc<SqliteStore>, run_id: &str, pass: bool, network: Option<Value>) {
    seed_lineage(store, run_id, pass, network, RunLineage::default()).await;
}

async fn seed_lineage(
    store: Arc<SqliteStore>,
    run_id: &str,
    pass: bool,
    network: Option<Value>,
    lineage: RunLineage,
) {
    let mut w = EvidenceWriter::begin_scoped_with_secrets_and_lineage(
        store,
        run_id,
        DEF,
        BTreeMap::new(),
        RunScope::default(),
        lineage.clone(),
        duhem_evidence::SecretRegistry::new(),
    )
    .await
    .unwrap();
    w.append(run_started_with_definition_and_lineage(
        DEF,
        BTreeMap::new(),
        None,
        lineage,
    ))
    .await
    .unwrap();
    // Two ui steps → two `ui` spans (the delivery-web layer chain).
    w.append(step_started(0, "ui/navigate")).await.unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 0,
        outcome: StepOutcome::Ok,
        detail: None,
    })
    .await
    .unwrap();
    w.append(step_started(1, "ui/assert-element"))
        .await
        .unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 1,
        outcome: StepOutcome::Ok,
        detail: None,
    })
    .await
    .unwrap();
    let (state, detail) = if pass {
        (VerdictState::Pass, None)
    } else {
        (
            VerdictState::Fail,
            Some("actual false, expected true".to_string()),
        )
    };
    w.append(EventPayload::AssertionEvaluated {
        operands: None,
        check_id: "AC-1.1".into(),
        iteration: None,
        assertion_index: 0,
        state,
        detail,
        expr: None,
        step_index: None,
    })
    .await
    .unwrap();
    if !pass {
        let shot = w.write_blob(&common::png_bytes()).await.unwrap();
        w.append(EventPayload::StepObservation {
            step_index: 1,
            output_name: "capture/screenshot".into(),
            value: ObservationValue::Blob {
                blob_sha256: shot.as_str().into(),
                mask_counts: BTreeMap::new(),
            },
        })
        .await
        .unwrap();
        if let Some(har) = network {
            let net = w
                .write_blob(&serde_json::to_vec(&har).unwrap())
                .await
                .unwrap();
            w.append(EventPayload::StepObservation {
                step_index: 1,
                output_name: "capture/network".into(),
                value: ObservationValue::Blob {
                    blob_sha256: net.as_str().into(),
                    mask_counts: BTreeMap::new(),
                },
            })
            .await
            .unwrap();
        }
    }
    let verdict = if pass {
        VerdictState::Pass
    } else {
        VerdictState::Fail
    };
    w.append(EventPayload::CheckFinished {
        gated_judging_steps: 0,
        check_id: "AC-1.1".into(),
        criterion_id: Some("AC-1".into()),
        verdict,
        session_source: None,
        session_digest: None,
    })
    .await
    .unwrap();
    w.append(EventPayload::CriterionFinished {
        criterion_id: "AC-1".into(),
        verdict,
    })
    .await
    .unwrap();
    w.append(EventPayload::RunFinished {
        body_verdict: None,
        verdict: Some(verdict),
    })
    .await
    .unwrap();
    w.finish().await.unwrap();
}

#[tokio::test]
async fn envelope_hands_an_agent_everything_to_react() {
    let (_tmp, rw, ro) = common::open_stores().await;
    seed(rw, RUN, false, Some(good_har())).await;

    let (status, json) =
        get_json(EvidenceReader::new(ro), &format!("/api/runs/{RUN}/failure")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["verdict"], "fail");
    let failing = json["failing"].as_array().unwrap();
    assert_eq!(failing.len(), 1);
    let fc = &failing[0];
    assert!(fc.get("run_id").is_none());
    assert!(fc.get("verification").is_none());
    assert_eq!(fc["criterion_id"], "AC-1");
    assert_eq!(fc["check_id"], "AC-1.1");
    // The delivery-web layer chain (#192).
    assert_eq!(fc["layers"], json!(["ui", "ui"]));
    // The failing assertion with its recorded cause.
    assert_eq!(fc["assertions"][0]["state"], "fail");
    assert_eq!(fc["assertions"][0]["detail"], "actual false, expected true");
    // Artifact URLs (screenshot + network), for the agent to fetch.
    let kinds: Vec<&str> = fc["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"capture/screenshot"));
    assert!(kinds.contains(&"capture/network"));
    // The first failing request, mined from the HAR.
    assert_eq!(fc["first_failing_request"]["method"], "POST");
    assert_eq!(fc["first_failing_request"]["status"], 500);
    let mut legacy_fc = fc.clone();
    legacy_fc
        .as_object_mut()
        .unwrap()
        .remove("gated_judging_steps");
    assert_eq!(
        serde_json::to_vec(&legacy_fc).unwrap(),
        serde_json::to_vec(&json!({
            "criterion_id": "AC-1",
            "check_id": "AC-1.1",
            "verdict": "fail",
            "layers": ["ui", "ui"],
            "assertions": [{
                "assertion_index": 0,
                "state": "fail",
                "detail": "actual false, expected true"
            }],
            "artifacts": [
                {"id": fc["artifacts"][0]["id"], "kind": "capture/screenshot", "url": format!("/api/runs/{RUN}/artifact/{}", fc["artifacts"][0]["id"])},
                {"id": fc["artifacts"][1]["id"], "kind": "capture/network", "url": format!("/api/runs/{RUN}/artifact/{}", fc["artifacts"][1]["id"])},
            ],
            "first_failing_request": fc["first_failing_request"],
        }))
        .unwrap()
    );
    assert!(
        fc["first_failing_request"]["url"]
            .as_str()
            .unwrap()
            .contains("/api/charge")
    );
}

#[tokio::test]
async fn parent_envelope_lists_descendant_failure_with_provenance() {
    let (_tmp, rw, ro) = common::open_stores().await;
    const ROOT: &str = "01FAILENVELOPE00000000000D";
    const CHILD: &str = "01FAILENVELOPE00000000000E";
    const LATER: &str = "01FAILENVELOPE00000000000H";
    seed_lineage(rw.clone(), ROOT, true, None, RunLineage::default()).await;
    seed_lineage(
        rw.clone(),
        LATER,
        true,
        None,
        RunLineage {
            parent_run_id: Some(ROOT.to_string()),
            origin: Some(RunOrigin::Suite),
        },
    )
    .await;
    seed_lineage(
        rw,
        CHILD,
        false,
        None,
        RunLineage {
            parent_run_id: Some(ROOT.to_string()),
            origin: Some(RunOrigin::Suite),
        },
    )
    .await;

    let (status, json) = get_json(
        EvidenceReader::new(ro),
        &format!("/api/runs/{ROOT}/failure"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["verdict"], "fail");
    assert_eq!(json["failing"].as_array().unwrap().len(), 1);
    assert_eq!(json["failing"][0]["run_id"], CHILD);
    assert_eq!(json["failing"][0]["verification"], "checkout");
    assert_eq!(json["failing"][0]["check_id"], "AC-1.1");
    assert!(
        json["failing"][0]["artifacts"][0]["url"]
            .as_str()
            .unwrap()
            .contains(CHILD)
    );
}

#[tokio::test]
async fn nested_invocation_child_is_not_part_of_the_tree() {
    // A failing control run started from inside a check (origin
    // `invocation`) stays that check's evidence; the leaf's own envelope
    // keeps its single-leaf shape and verdict.
    let (_tmp, rw, ro) = common::open_stores().await;
    const LEAF: &str = "01FAILENVELOPE00000000000J";
    const NESTED: &str = "01FAILENVELOPE00000000000K";
    seed_lineage(rw.clone(), LEAF, true, None, RunLineage::default()).await;
    seed_lineage(
        rw,
        NESTED,
        false,
        None,
        RunLineage {
            parent_run_id: Some(LEAF.to_string()),
            origin: Some(RunOrigin::Invocation),
        },
    )
    .await;

    let (status, json) = get_json(
        EvidenceReader::new(ro),
        &format!("/api/runs/{LEAF}/failure"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["verdict"], "pass");
    assert_eq!(json["failing"], json!([]));
}

#[tokio::test]
async fn passing_parent_tree_has_no_failing_checks() {
    let (_tmp, rw, ro) = common::open_stores().await;
    const ROOT: &str = "01FAILENVELOPE00000000000F";
    const CHILD: &str = "01FAILENVELOPE00000000000G";
    seed_lineage(rw.clone(), ROOT, true, None, RunLineage::default()).await;
    seed_lineage(
        rw,
        CHILD,
        true,
        None,
        RunLineage {
            parent_run_id: Some(ROOT.to_string()),
            origin: Some(RunOrigin::Suite),
        },
    )
    .await;

    let (status, json) = get_json(
        EvidenceReader::new(ro),
        &format!("/api/runs/{ROOT}/failure"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["failing"], json!([]));
}

#[tokio::test]
async fn a_passing_run_has_no_failing_checks() {
    let (_tmp, rw, ro) = common::open_stores().await;
    seed(rw, PASS, true, None).await;
    let (status, json) = get_json(
        EvidenceReader::new(ro),
        &format!("/api/runs/{PASS}/failure"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["verdict"], "pass");
    assert_eq!(json["failing"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn per_check_variant_scopes_to_one_check() {
    let (_tmp, rw, ro) = common::open_stores().await;
    seed(rw, RUN, false, Some(good_har())).await;
    let (status, json) = get_json(
        EvidenceReader::new(ro),
        &format!("/api/runs/{RUN}/failure/AC-1::AC-1.1"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["check_id"], "AC-1.1");
    assert_eq!(json["first_failing_request"]["status"], 500);
}

#[tokio::test]
async fn malformed_failing_request_is_omitted_not_emitted_empty() {
    let (_tmp, rw, ro) = common::open_stores().await;
    // The only ≥400 entry lacks a `request.url` — skip it rather than
    // emit an empty method/url that would mislead the agent.
    let bad = json!({ "log": { "entries": [
        { "request": { "method": "POST" }, "response": { "status": 500 } },
    ] } });
    seed(rw, "01FAILENVELOPE00000000000C", false, Some(bad)).await;
    let (status, json) = get_json(
        EvidenceReader::new(ro),
        "/api/runs/01FAILENVELOPE00000000000C/failure",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // No usable request → the field is omitted (never `{method:"", url:""}`).
    assert!(json["failing"][0]["first_failing_request"].is_null());
}

#[tokio::test]
async fn envelope_of_unknown_run_is_404() {
    let (_tmp, _rw, ro) = common::open_stores().await;
    let (status, _) = get_json(
        EvidenceReader::new(ro),
        "/api/runs/01ZZZZZZZZZZZZZZZZZZZZZZZZZ/failure",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
