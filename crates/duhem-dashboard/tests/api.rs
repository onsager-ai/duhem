//! Integration tests for the #85 JSON API: every endpoint, against
//! fixture evidence dirs written by the production `EvidenceWriter`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use duhem_dashboard::{EvidenceReader, load_run, router};
use duhem_evidence::{Trace, replay};
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
async fn runs_list_empty_dir_is_empty_array() {
    let tmp = tempfile::tempdir().unwrap();
    let (status, json) = get_json(EvidenceReader::new(tmp.path()), "/api/runs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json, serde_json::json!([]));
}

#[tokio::test]
async fn runs_list_single_leaf_run() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000A");
    common::write_passing_run(&run_dir, "verifications/create-workspace.yml");

    let (status, json) = get_json(EvidenceReader::new(tmp.path()), "/api/runs").await;
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
async fn runs_list_nests_run_set_leaves_and_rolls_up_with_the_judge_fold() {
    let tmp = tempfile::tempdir().unwrap();
    common::write_passing_run(
        &tmp.path().join("login/01J0000000000000000000000A"),
        "verifications/login/duhem.yml",
    );
    common::write_failing_run(
        &tmp.path().join("login/01J0000000000000000000000B"),
        "verifications/login/duhem.yml",
    );

    let (status, json) = get_json(EvidenceReader::new(tmp.path()), "/api/runs").await;
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
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000C");
    common::write_in_progress_run(&run_dir, "verifications/live.yml");

    let (_, json) = get_json(EvidenceReader::new(tmp.path()), "/api/runs").await;
    let row = &json.as_array().unwrap()[0];
    assert_eq!(row["live"], true);
    assert_eq!(row["verdict"], Value::Null);
    assert_eq!(row["duration_ms"], Value::Null);
}

#[tokio::test]
async fn run_detail_carries_inputs_verdict_and_criteria() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000A");
    common::write_passing_run(&run_dir, "verifications/create-workspace.yml");

    let (status, json) = get_json(
        EvidenceReader::new(tmp.path()),
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
    let tmp = tempfile::tempdir().unwrap();
    let (status, _) = get_json(
        EvidenceReader::new(tmp.path()),
        "/api/runs/01J0000000000000000000000Z",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn aborted_setup_run_surfaces_the_abort() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000D");
    common::write_aborted_run(&run_dir, "verifications/aborted.yml");

    let (status, json) = get_json(
        EvidenceReader::new(tmp.path()),
        "/api/runs/01J0000000000000000000000D",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["setup_aborted"], true);
    assert_eq!(json["verdict"], "inconclusive:environment_error");
    assert_eq!(json["criteria"], serde_json::json!([]));
}

#[tokio::test]
async fn check_detail_timeline_matches_trace_order_and_lists_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000A");
    let sha = common::write_passing_run(&run_dir, "verifications/create-workspace.yml");

    let (status, json) = get_json(
        EvidenceReader::new(tmp.path()),
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
    // Trace order == seq order: the timeline is a filter over the
    // trace, never a re-sort.
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
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000A");
    common::write_passing_run(&run_dir, "verifications/x.yml");
    let reader = EvidenceReader::new(tmp.path());

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
async fn raw_trace_is_served_byte_identical() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000A");
    common::write_passing_run(&run_dir, "verifications/x.yml");
    let on_disk = std::fs::read(run_dir.join("trace.jsonl")).unwrap();

    let (status, body, content_type) = get(
        EvidenceReader::new(tmp.path()),
        "/api/runs/01J0000000000000000000000A/trace.jsonl",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, on_disk);
    assert_eq!(content_type, "application/x-ndjson");
}

#[tokio::test]
async fn artifact_serves_bytes_with_sniffed_content_type() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000A");
    let sha = common::write_passing_run(&run_dir, "verifications/x.yml");
    let reader = EvidenceReader::new(tmp.path());

    let (status, body, content_type) = get(
        reader.clone(),
        &format!("/api/runs/01J0000000000000000000000A/artifact/{sha}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "image/png");
    assert_eq!(body, common::png_bytes());

    // Path-shaped ids are rejected before touching the filesystem.
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
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000A");
    common::write_passing_run(&run_dir, "verifications/x.yml");
    let reader = EvidenceReader::new(tmp.path());

    let (s1, plain) = get_json(reader.clone(), "/api/runs/01J0000000000000000000000A").await;
    let (s2, suffixed) = get_json(reader, "/api/runs/01J0000000000000000000000A.json").await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(plain, suffixed);
}

#[tokio::test]
async fn spa_index_is_served_at_root_and_as_deep_link_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let reader = EvidenceReader::new(tmp.path());

    let (status, body, content_type) = get(reader.clone(), "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/html"));
    assert!(!body.is_empty());

    // Unknown non-/api path falls back to the SPA index.
    let (status, fallback, _) = get(reader, "/some/client/route").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fallback, body);
}

/// PR #88 review: a run with no parseable `started_at` (empty trace)
/// must sort to the bottom of the list, not float to the top.
#[tokio::test]
async fn runs_with_no_timestamp_sort_last() {
    let tmp = tempfile::tempdir().unwrap();
    let empty_dir = tmp.path().join("01J0000000000000000000000E");
    std::fs::create_dir_all(&empty_dir).unwrap();
    std::fs::write(empty_dir.join("trace.jsonl"), b"").unwrap();
    common::write_passing_run(
        &tmp.path().join("01J0000000000000000000000A"),
        "verifications/x.yml",
    );

    let (_, json) = get_json(EvidenceReader::new(tmp.path()), "/api/runs").await;
    let rows = json.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["run_id"], "01J0000000000000000000000A");
    assert_eq!(rows[1]["run_id"], "01J0000000000000000000000E");
    assert_eq!(rows[1]["started_at"], Value::Null);
}

/// PR #88 review: when a malformed trace reuses one check id under
/// two criteria, the first `step_started` owns it — the check is
/// listed once, its verdict lands only there, and the colliding pair
/// is not addressable.
#[tokio::test]
async fn colliding_check_ids_attribute_to_the_first_owner() {
    use duhem_evidence::{EventPayload, EvidenceWriter, StepOutcome, VerdictState, run_started};
    use std::collections::BTreeMap;

    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000F");
    let mut w = EvidenceWriter::new(&run_dir, "verifications/dup.yml").unwrap();
    w.append(run_started("verifications/dup.yml", BTreeMap::new()))
        .unwrap();
    for criterion in ["AC-1", "AC-2"] {
        w.append(EventPayload::StepStarted {
            criterion_id: criterion.into(),
            check_id: "DUP".into(),
            step_index: 0,
            uses: "api/call".into(),
            with: BTreeMap::new(),
        })
        .unwrap();
        w.append(EventPayload::StepFinished {
            step_index: 0,
            outcome: StepOutcome::Ok,
        })
        .unwrap();
    }
    w.append(EventPayload::CheckFinished {
        check_id: "DUP".into(),
        verdict: VerdictState::Pass,
    })
    .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Pass,
    })
    .unwrap();
    w.finish().unwrap();

    let reader = EvidenceReader::new(tmp.path());
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
#[test]
fn reader_verdict_matches_replay() {
    let tmp = tempfile::tempdir().unwrap();
    for (dir, def) in [
        ("01J0000000000000000000000A", "verifications/pass.yml"),
        ("01J0000000000000000000000B", "verifications/fail.yml"),
    ] {
        let run_dir = tmp.path().join(dir);
        if def.contains("pass") {
            common::write_passing_run(&run_dir, def);
        } else {
            common::write_failing_run(&run_dir, def);
        }
        let evidence = load_run(&run_dir).unwrap();
        let trace = Trace::open(&run_dir).unwrap();
        let replayed = replay(&trace).expect("fixture must replay cleanly");
        assert_eq!(evidence.verdict(), Some(replayed.run.state));
    }
}
