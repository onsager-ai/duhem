//! `GET /api/runs/:id/diff` (#211): the run-to-run regression diff.
//! Seeds a passing baseline + a failing current run of the same
//! verification/target and asserts the diff resolves the last-pass
//! baseline and surfaces the flipped criterion/check/assertion, plus
//! the honest `baseline: null` when nothing passed.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use duhem_dashboard::{EvidenceReader, router};
use duhem_evidence::{
    EventPayload, EvidenceWriter, ObservationValue, RunScope, SqliteStore, StepOutcome,
    VerdictState, run_started,
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const BASE: &str = "01AAAAAAAAAAAAAAAAAAAAAAAB";
const CURR: &str = "01AAAAAAAAAAAAAAAAAAAAAAAC";
const DEF: &str = "verifications/checkout.yml";
const TARGET: &str = "github.com/acme/app";

async fn get_json(reader: EvidenceReader, path: &str) -> (StatusCode, Value) {
    let res = router(reader)
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// One run of the `checkout` verification: criterion AC-1 / check
/// AC-1.1 / one assertion. `pass` drives the verdict + assertion
/// state; `screenshot` records a `capture/screenshot` blob.
async fn seed(store: Arc<SqliteStore>, run_id: &str, pass: bool, screenshot: bool) {
    let scope = RunScope {
        project_id: Some(TARGET.into()),
        verifier_repo: None,
        verifier_sha: None,
        target_repo: Some(TARGET.into()),
        target_sha: Some(if pass { "sha-good" } else { "sha-bad" }.into()),
    };
    let mut w = EvidenceWriter::begin_scoped(store, run_id, DEF, BTreeMap::new(), scope)
        .await
        .unwrap();
    w.append(run_started(DEF, BTreeMap::new())).await.unwrap();
    w.append(EventPayload::StepStarted {
        criterion_id: "AC-1".into(),
        check_id: "AC-1.1".into(),
        step_index: 0,
        uses: "ui/assert-element".into(),
        layer: Some("ui".into()),
        with: BTreeMap::new(),
    })
    .await
    .unwrap();
    w.append(EventPayload::StepFinished {
        step_index: 0,
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
        step_index: None,
    })
    .await
    .unwrap();
    if screenshot {
        let sha = w.write_blob(&common::png_bytes()).await.unwrap();
        w.append(EventPayload::StepObservation {
            step_index: 0,
            output_name: "capture/screenshot".into(),
            value: ObservationValue::Blob {
                blob_sha256: sha.as_str().to_string(),
            },
        })
        .await
        .unwrap();
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
async fn diff_surfaces_the_regression_against_the_last_pass() {
    let (_tmp, rw, ro) = common::open_stores().await;
    seed(rw.clone(), BASE, true, false).await; // baseline: passed
    seed(rw.clone(), CURR, false, true).await; // current: failed, with a screenshot

    let (status, json) = get_json(EvidenceReader::new(ro), &format!("/api/runs/{CURR}/diff")).await;
    assert_eq!(status, StatusCode::OK);

    // The last-pass baseline is auto-resolved (same verification + target).
    assert_eq!(json["current"]["run_id"], CURR);
    assert_eq!(json["current"]["verdict"], "fail");
    assert_eq!(json["baseline"]["run_id"], BASE);
    assert_eq!(json["baseline"]["verdict"], "pass");

    // The regression is surfaced top-to-bottom: criterion → check →
    // assertion, each marked changed with the recorded transition.
    let crit = &json["criteria"][0];
    assert_eq!(crit["id"], "AC-1");
    assert_eq!(crit["changed"], true);
    assert_eq!(crit["baseline_verdict"], "pass");
    assert_eq!(crit["current_verdict"], "fail");

    let check = &crit["checks"][0];
    assert_eq!(check["id"], "AC-1.1");
    assert_eq!(check["changed"], true);

    let a = &check["assertions"][0];
    assert_eq!(a["changed"], true);
    assert_eq!(a["baseline_state"], "pass");
    assert_eq!(a["current_state"], "fail");
    assert_eq!(a["current_detail"], "actual false, expected true");

    // Artifacts on both sides: current has the screenshot, baseline none.
    assert_eq!(check["current_artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(check["current_artifacts"][0]["kind"], "capture/screenshot");
    assert_eq!(check["baseline_artifacts"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn diff_reports_no_baseline_when_the_verification_never_passed() {
    let (_tmp, rw, ro) = common::open_stores().await;
    seed(rw.clone(), CURR, false, false).await; // only a failing run exists

    let (status, json) = get_json(EvidenceReader::new(ro), &format!("/api/runs/{CURR}/diff")).await;
    assert_eq!(status, StatusCode::OK);
    // Honest: no prior passing run → no comparison, nothing painted changed.
    assert!(json["baseline"].is_null());
    assert_eq!(json["criteria"][0]["changed"], false);
    assert_eq!(json["criteria"][0]["checks"][0]["changed"], false);
}

#[tokio::test]
async fn diff_baseline_override_pins_a_specific_run() {
    let (_tmp, rw, ro) = common::open_stores().await;
    seed(rw.clone(), BASE, true, false).await;
    seed(rw.clone(), CURR, false, false).await;

    let (status, json) = get_json(
        EvidenceReader::new(ro),
        &format!("/api/runs/{CURR}/diff?baseline={BASE}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["baseline"]["run_id"], BASE);
    assert_eq!(json["criteria"][0]["changed"], true);
}

#[tokio::test]
async fn diff_of_unknown_run_is_404() {
    let (_tmp, _rw, ro) = common::open_stores().await;
    let (status, _) = get_json(
        EvidenceReader::new(ro),
        "/api/runs/01ZZZZZZZZZZZZZZZZZZZZZZZZZ/diff",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
