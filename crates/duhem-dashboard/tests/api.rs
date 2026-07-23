//! Integration tests for the #85 JSON API: every endpoint, against
//! fixture runs written into a real store by the production
//! `EvidenceWriter` (#189).

mod common;

use std::collections::BTreeMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use duhem_dashboard::{EvidenceReader, events_to_jsonl, router};
use duhem_evidence::{
    EventPayload, EvidenceWriter, Store, Trace, VerdictState, replay, run_started_with_definition,
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

async fn get(reader: EvidenceReader, path: &str) -> (StatusCode, Vec<u8>, String) {
    let app = router(reader);
    let res = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let content_type = res
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or_default().to_string())
        .unwrap_or_default();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, body, content_type)
}

async fn get_json(reader: EvidenceReader, path: &str) -> (StatusCode, Value) {
    let (status, body, _) = get(reader, path).await;
    let json = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test]
async fn runs_list_empty_store_is_empty_array() {
    let (_tmp, _rw, ro) = common::open_stores().await;
    let (status, json) = get_json(EvidenceReader::new(ro), "/api/runs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json, serde_json::json!([]));
}

#[tokio::test]
async fn runs_list_single_leaf_run() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(
        rw,
        "01J0000000000000000000000A",
        "verifications/create-workspace.yml",
    )
    .await;

    let (status, json) = get_json(EvidenceReader::new(ro), "/api/runs").await;
    assert_eq!(status, StatusCode::OK);
    let rows = json.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["run_id"], "01J0000000000000000000000A");
    assert_eq!(row["verification"], "create-workspace");
    assert_eq!(row["verdict"], "pass");
    assert_eq!(row["kind"], "leaf");
    assert_eq!(row["live"], false);
    assert!(row["started_at"].is_string());
    assert!(row["duration_ms"].is_u64());
}

#[tokio::test]
async fn runs_list_groups_a_verifications_runs_and_rolls_up_with_the_judge_fold() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(
        rw.clone(),
        "01J0000000000000000000000A",
        "verifications/login/duhem.yml",
    )
    .await;
    common::write_failing_run(
        rw,
        "01J0000000000000000000000B",
        "verifications/login/duhem.yml",
    )
    .await;

    let (status, json) = get_json(EvidenceReader::new(ro), "/api/runs").await;
    assert_eq!(status, StatusCode::OK);
    let rows = json.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let set = &rows[0];
    assert_eq!(set["kind"], "run-set");
    assert_eq!(set["verification"], "login");
    // Any fail → fail (issue #49's rule, applied via aggregate_run_set).
    assert_eq!(set["verdict"], "fail");
    let children = set["children"].as_array().unwrap();
    assert_eq!(children.len(), 2);
    assert!(children.iter().all(|c| c["kind"] == "leaf"));
    assert!(children.iter().all(|c| c["verification"] == "login"));
}

#[tokio::test]
async fn in_progress_run_is_live_with_no_verdict() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_in_progress_run(rw, "01J0000000000000000000000C", "verifications/live.yml").await;

    let (_, json) = get_json(EvidenceReader::new(ro), "/api/runs").await;
    let row = &json.as_array().unwrap()[0];
    assert_eq!(row["live"], true);
    assert_eq!(row["verdict"], Value::Null);
    assert_eq!(row["duration_ms"], Value::Null);
}

#[tokio::test]
async fn run_detail_carries_inputs_verdict_and_criteria() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(
        rw,
        "01J0000000000000000000000A",
        "verifications/create-workspace.yml",
    )
    .await;

    let (status, json) = get_json(
        EvidenceReader::new(ro),
        "/api/runs/01J0000000000000000000000A",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["run_id"], "01J0000000000000000000000A");
    assert_eq!(json["verification"], "create-workspace");
    assert_eq!(json["inputs"]["workspace_name"], "ws-fixture");
    assert_eq!(json["verdict"], "pass");
    assert_eq!(json["setup_aborted"], false);
    let criteria = json["criteria"].as_array().unwrap();
    assert_eq!(criteria.len(), 1);
    assert_eq!(criteria[0]["id"], "AC-1");
    assert_eq!(criteria[0]["verdict"], "pass");
    let checks = criteria[0]["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0]["id"], "AC-1.1");
    assert_eq!(checks[0]["verdict"], "pass");
}

#[tokio::test]
async fn run_detail_unknown_run_is_404() {
    let (_tmp, _rw, ro) = common::open_stores().await;
    let (status, _) = get_json(
        EvidenceReader::new(ro),
        "/api/runs/01J0000000000000000000000Z",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn aborted_setup_run_surfaces_the_abort() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_aborted_run(
        rw,
        "01J0000000000000000000000D",
        "verifications/aborted.yml",
    )
    .await;

    let (status, json) = get_json(
        EvidenceReader::new(ro),
        "/api/runs/01J0000000000000000000000D",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["setup_aborted"], true);
    assert_eq!(json["verdict"], "inconclusive:environment_error");
    assert_eq!(json["criteria"], serde_json::json!([]));
}

#[tokio::test]
async fn check_detail_timeline_matches_stream_order_and_lists_artifacts() {
    let (_tmp, rw, ro) = common::open_stores().await;
    let sha = common::write_passing_run(
        rw,
        "01J0000000000000000000000A",
        "verifications/create-workspace.yml",
    )
    .await;

    let (status, json) = get_json(
        EvidenceReader::new(ro),
        "/api/runs/01J0000000000000000000000A/checks/AC-1::AC-1.1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["criterion_id"], "AC-1");
    assert_eq!(json["check_id"], "AC-1.1");
    assert_eq!(json["verdict"], "pass");

    let timeline = json["timeline"].as_array().unwrap();
    let kinds: Vec<&str> = timeline
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "step_started",
            "step_observation",
            "step_finished",
            "step_started",
            "step_observation",
            "step_finished",
            "assertion_evaluated",
            "check_finished",
        ]
    );
    // Stream order == seq order: the timeline is a filter over the
    // stream, never a re-sort.
    let seqs: Vec<u64> = timeline
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));

    let artifacts = json["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0]["id"], sha.as_str());
    assert_eq!(artifacts[0]["kind"], "screenshot");
    assert_eq!(
        artifacts[0]["url"],
        format!("/api/runs/01J0000000000000000000000A/artifact/{sha}")
    );
}

#[tokio::test]
async fn check_detail_unknown_pair_is_404_and_bad_pair_is_400() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(rw, "01J0000000000000000000000A", "verifications/x.yml").await;
    let reader = EvidenceReader::new(ro);

    let (status, _) = get_json(
        reader.clone(),
        "/api/runs/01J0000000000000000000000A/checks/AC-9::AC-9.9",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = get_json(
        reader,
        "/api/runs/01J0000000000000000000000A/checks/not-a-pair",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn raw_trace_is_served_as_the_wire_format_stream() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(rw, "01J0000000000000000000000A", "verifications/x.yml").await;
    let expected = events_to_jsonl(&ro.run_events("01J0000000000000000000000A").await.unwrap());

    let (status, body, content_type) = get(
        EvidenceReader::new(ro),
        "/api/runs/01J0000000000000000000000A/trace.jsonl",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, expected.as_bytes());
    assert_eq!(content_type, "application/x-ndjson");
}

#[tokio::test]
async fn run_definition_is_served_verbatim_and_flagged_on_run_detail() {
    let (_tmp, rw, ro) = common::open_stores().await;
    let yaml = "verification: t\ncriteria: []\n";
    let mut w = EvidenceWriter::begin(
        rw,
        "01J0000000000000000000000A",
        "verifications/x.yml",
        BTreeMap::new(),
    )
    .await
    .expect("writer");
    w.append(run_started_with_definition(
        "verifications/x.yml",
        BTreeMap::new(),
        Some(yaml.to_string()),
    ))
    .await
    .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Pass,
    })
    .await
    .unwrap();
    w.finish().await.unwrap();
    let reader = EvidenceReader::new(ro);

    let (status, json) = get_json(reader.clone(), "/api/runs/01J0000000000000000000000A").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["has_definition"], true);

    let (status, body, content_type) =
        get(reader, "/api/runs/01J0000000000000000000000A/definition").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, yaml.as_bytes());
    assert_eq!(content_type, "application/x-yaml; charset=utf-8");
}

#[tokio::test]
async fn run_definition_absent_is_404_and_flagged_false_on_run_detail() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(rw, "01J0000000000000000000000A", "verifications/x.yml").await;
    let reader = EvidenceReader::new(ro);

    let (status, json) = get_json(reader.clone(), "/api/runs/01J0000000000000000000000A").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["has_definition"], false);

    let (status, _, _) = get(reader, "/api/runs/01J0000000000000000000000A/definition").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn artifact_serves_bytes_with_sniffed_content_type() {
    let (_tmp, rw, ro) = common::open_stores().await;
    let sha =
        common::write_passing_run(rw, "01J0000000000000000000000A", "verifications/x.yml").await;
    let reader = EvidenceReader::new(ro);

    let (status, body, content_type) = get(
        reader.clone(),
        &format!("/api/runs/01J0000000000000000000000A/artifact/{sha}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "image/png");
    assert_eq!(body, common::png_bytes());

    // Path-shaped ids are rejected before touching the store.
    let (status, _, _) = get(
        reader.clone(),
        "/api/runs/01J0000000000000000000000A/artifact/not-a-sha",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let missing = "0".repeat(64);
    let (status, _, _) = get(
        reader,
        &format!("/api/runs/01J0000000000000000000000A/artifact/{missing}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn json_suffix_aliases_serve_the_same_shapes() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(rw, "01J0000000000000000000000A", "verifications/x.yml").await;
    let reader = EvidenceReader::new(ro);

    let (s1, plain) = get_json(reader.clone(), "/api/runs/01J0000000000000000000000A").await;
    let (s2, suffixed) = get_json(reader, "/api/runs/01J0000000000000000000000A.json").await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(plain, suffixed);
}

#[tokio::test]
async fn spa_index_is_served_at_root_and_as_deep_link_fallback() {
    let (_tmp, _rw, ro) = common::open_stores().await;
    let reader = EvidenceReader::new(ro);

    let (status, body, content_type) = get(reader.clone(), "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/html"));
    assert!(!body.is_empty());

    // Unknown non-/api path falls back to the SPA index.
    let (status, fallback, _) = get(reader, "/some/client/route").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fallback, body);
}

/// PR #88 review: when a malformed stream reuses one check id under
/// two criteria, the first `step_started` owns it — the check is
/// listed once, its verdict lands only there, and the colliding pair
/// is not addressable.
#[tokio::test]
async fn colliding_check_ids_attribute_to_the_first_owner() {
    use duhem_evidence::{EventPayload, EvidenceWriter, StepOutcome, VerdictState, run_started};
    use std::collections::BTreeMap;

    let (_tmp, rw, ro) = common::open_stores().await;
    let mut w = EvidenceWriter::begin(
        rw,
        "01J0000000000000000000000F",
        "verifications/dup.yml",
        BTreeMap::new(),
    )
    .await
    .unwrap();
    w.append(run_started("verifications/dup.yml", BTreeMap::new()))
        .await
        .unwrap();
    for criterion in ["AC-1", "AC-2"] {
        w.append(EventPayload::StepStarted {
            criterion_id: criterion.into(),
            check_id: "DUP".into(),
            step_index: 0,
            uses: "api/call".into(),
            layer: None,
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
    }
    w.append(EventPayload::CheckFinished {
        check_id: "DUP".into(),
        verdict: VerdictState::Pass,
    })
    .await
    .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Pass,
    })
    .await
    .unwrap();
    w.finish().await.unwrap();

    let reader = EvidenceReader::new(ro);
    let (_, detail) = get_json(reader.clone(), "/api/runs/01J0000000000000000000000F").await;
    let criteria = detail["criteria"].as_array().unwrap();
    let ac1 = criteria.iter().find(|c| c["id"] == "AC-1").unwrap();
    assert_eq!(ac1["checks"][0]["id"], "DUP");
    assert_eq!(ac1["checks"][0]["verdict"], "pass");
    // The colliding second criterion does not list (or get the
    // verdict of) the check it doesn't own.
    if let Some(ac2) = criteria.iter().find(|c| c["id"] == "AC-2") {
        assert!(ac2["checks"].as_array().unwrap().is_empty());
    }

    let (status, _) = get_json(
        reader.clone(),
        "/api/runs/01J0000000000000000000000F/checks/AC-2::DUP",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, check) = get_json(
        reader,
        "/api/runs/01J0000000000000000000000F/checks/AC-1::DUP",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(check["verdict"], "pass");
}

/// #85 Test bullet: the verdict the reader surfaces is the verdict
/// `duhem_evidence::replay` reconstructs — the dashboard shows the
/// judge's verdict, never its own.
#[tokio::test]
async fn reader_verdict_matches_replay() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(
        rw.clone(),
        "01J0000000000000000000000A",
        "verifications/pass.yml",
    )
    .await;
    common::write_failing_run(rw, "01J0000000000000000000000B", "verifications/fail.yml").await;

    for run_id in ["01J0000000000000000000000A", "01J0000000000000000000000B"] {
        let record = ro.get_run(run_id).await.unwrap().unwrap();
        let trace = Trace::from_store(ro.as_ref(), run_id).await.unwrap();
        let replayed = replay(&trace).expect("fixture must replay cleanly");
        assert_eq!(record.verdict, Some(replayed.run.state));
    }
}

/// #193: the check detail carries the ④ span chain folded from the
/// run's layer tags, each span linking back to its evidence seq.
#[tokio::test]
async fn check_detail_carries_the_delivery_web_spans() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(rw, "01J0000000000000000000000A", "verifications/x.yml").await;

    let (status, json) = get_json(
        EvidenceReader::new(ro),
        "/api/runs/01J0000000000000000000000A/checks/AC-1::AC-1.1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let spans = json["spans"].as_array().unwrap();
    assert_eq!(spans.len(), 2, "two tagged ui steps: {spans:?}");
    assert!(spans.iter().all(|s| s["layer"] == "ui" && s["ok"] == true));
    // Evidence linkage: span seq points at a real timeline event.
    let seqs: Vec<u64> = json["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert!(
        spans
            .iter()
            .all(|s| seqs.contains(&s["seq"].as_u64().unwrap()))
    );
}

/// #193 ②: the verification-history endpoint returns the runs axis
/// (newest first) and each criterion's verdict across them.
#[tokio::test]
async fn verification_history_returns_the_criterion_spine() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(
        rw.clone(),
        "01J0000000000000000000000A",
        "verifications/login/duhem.yml",
    )
    .await;
    common::write_failing_run(
        rw,
        "01J0000000000000000000000B",
        "verifications/login/duhem.yml",
    )
    .await;

    let (status, json) = get_json(
        EvidenceReader::new(ro.clone()),
        "/api/verifications/login/history",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["name"], "login");
    let runs = json["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    // Newest first: B was written after A.
    assert_eq!(runs[0]["run_id"], "01J0000000000000000000000B");
    assert_eq!(runs[0]["verdict"], "fail");
    assert_eq!(runs[1]["verdict"], "pass");
    let criteria = json["criteria"].as_array().unwrap();
    assert_eq!(criteria.len(), 1);
    assert_eq!(criteria[0]["criterion_id"], "AC-1");
    assert_eq!(
        criteria[0]["verdicts"],
        serde_json::json!(["fail", "pass"]),
        "criterion verdicts follow the runs axis"
    );

    // Unknown verification → 404.
    let (status, _) = get_json(EvidenceReader::new(ro), "/api/verifications/nope/history").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
