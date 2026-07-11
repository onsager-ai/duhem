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
    EventPayload, EvidenceWriter, ObservationValue, SqliteStore, StepOutcome, VerdictState,
    run_started,
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
        with: BTreeMap::new(),
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
    let mut w = EvidenceWriter::begin(store, run_id, DEF, BTreeMap::new())
        .await
        .unwrap();
    w.append(run_started(DEF, BTreeMap::new())).await.unwrap();
    // Two ui steps → two `ui` spans (the delivery-web layer chain).
    w.append(step_started(0, "ui/navigate")).await.unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 0,
        outcome: StepOutcome::Ok,
    })
    .await
    .unwrap();
    w.append(step_started(1, "ui/assert-element"))
        .await
        .unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 1,
        outcome: StepOutcome::Ok,
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
        check_id: "AC-1.1".into(),
        assertion_index: 0,
        state,
        detail,
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
        check_id: "AC-1.1".into(),
        verdict,
    })
    .await
    .unwrap();
    w.append(EventPayload::CriterionFinished {
        criterion_id: "AC-1".into(),
        verdict,
    })
    .await
    .unwrap();
    w.append(EventPayload::RunFinished { verdict })
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
    assert!(
        fc["first_failing_request"]["url"]
            .as_str()
            .unwrap()
            .contains("/api/charge")
    );
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
