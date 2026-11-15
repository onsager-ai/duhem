//! Native gates and navigation on dashboard run trees (#649).
mod common;

use std::{collections::BTreeMap, sync::Arc};

use duhem_dashboard::EvidenceReader;
use duhem_evidence::{
    EventPayload, EvidenceWriter, RunLineage, RunOrigin, RunScope, SecretRegistry, SqliteStore,
    VerdictState, run_started_with_definition_and_lineage,
};
use duhem_summary::NativeRunSummary;

async fn seed(store: Arc<SqliteStore>, id: &str, parent: Option<&str>, required: Option<bool>) {
    let lineage = RunLineage {
        parent_run_id: parent.map(str::to_owned),
        origin: parent.map(|_| RunOrigin::Suite),
    };
    let mut secrets = SecretRegistry::new();
    secrets.register("token", "native-secret-value");
    let mut writer = EvidenceWriter::begin_scoped_with_secrets_and_lineage(
        store,
        id,
        "verifications/native.yml",
        BTreeMap::new(),
        RunScope::default(),
        lineage.clone(),
        secrets,
    )
    .await
    .unwrap();
    writer
        .append(run_started_with_definition_and_lineage(
            "verifications/native.yml",
            BTreeMap::new(),
            None,
            lineage,
        ))
        .await
        .unwrap();
    if let Some(required) = required {
        let sha = writer
            .write_blob(b"masked native evidence")
            .await
            .unwrap()
            .0;
        let result: NativeRunSummary = serde_json::from_value(serde_json::json!({
            "id": "lint", "layer": "static", "outcome": "fail", "duration_ms": 12,
            "required": required, "tests": 1, "exit_code": 1,
            "detail": writer.mask_text(&format!("native-secret-value {}", "x".repeat(3000))),
            "counts": {"passed": 0, "failed": 1, "errored": 0, "skipped": 0},
            "failing_tests": [{"test_id": "lint:test", "name": "test", "message": "masked failure"}],
            "stdout_blob_sha256": sha, "stderr_blob_sha256": sha,
            "report_blob_sha256": sha, "results_blob_sha256": sha,
        }))
        .unwrap();
        writer
            .append(EventPayload::NativeRunFinished { result })
            .await
            .unwrap();
    }
    writer
        .append(EventPayload::CheckFinished {
            gated_judging_steps: 0,
            check_id: "AC-1.1".into(),
            criterion_id: Some("AC-1".into()),
            verdict: VerdictState::Pass,
            session_source: None,
            session_digest: None,
        })
        .await
        .unwrap();
    writer
        .append(EventPayload::CriterionFinished {
            criterion_id: "AC-1".into(),
            verdict: VerdictState::Pass,
        })
        .await
        .unwrap();
    writer
        .append(EventPayload::RunFinished {
            body_verdict: None,
            verdict: Some(VerdictState::Pass),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn envelope_native_contract_and_static_evidence() {
    let (_tmp, rw, ro) = common::open_stores().await;
    seed(rw.clone(), "root", None, Some(false)).await;
    seed(rw, "leaf", Some("root"), Some(false)).await;
    let reader = EvidenceReader::new(ro);
    let detail = reader.run_detail("root").await.unwrap().unwrap();
    assert_eq!(detail.native.len(), 2);
    assert_eq!(
        detail
            .layers
            .iter()
            .find(|row| row.layer == "static")
            .unwrap()
            .failed,
        2
    );
    assert_eq!(
        detail
            .layers
            .iter()
            .find(|row| row.layer == "static")
            .unwrap()
            .duration_ms,
        24
    );
    let envelope = reader.failure_envelope("root").await.unwrap().unwrap();
    let json = serde_json::to_value(&envelope).unwrap();
    assert_eq!(json["verdict"], "pass");
    assert_eq!(json["failing_native"].as_array().unwrap().len(), 2);
    let failure = &json["failing_native"][0];
    assert_eq!(failure["run_id"], "root");
    assert_eq!(failure["native_id"], "lint");
    assert_eq!(failure["layer"], "static");
    assert_eq!(failure["outcome"], "fail");
    assert_eq!(failure["required"], false);
    let detail = failure["detail"].as_str().unwrap();
    assert!(!detail.contains("native-secret-value"));
    assert!(detail.ends_with("… [truncated]"));
    assert!(detail.chars().count() < 2100);
    assert_eq!(failure["failing_tests"][0]["test_id"], "lint:test");
    for (index, kind) in ["stdout", "stderr", "report", "results"].iter().enumerate() {
        let evidence = &failure["evidence"][index];
        assert_eq!(evidence["kind"], *kind);
        let sha = evidence["id"].as_str().unwrap();
        assert_eq!(evidence["url"], format!("/api/runs/root/artifact/{sha}"));
        assert_eq!(
            reader.artifact("root", sha).await.unwrap().unwrap().0,
            b"masked native evidence"
        );
    }
    let out = tempfile::tempdir().unwrap();
    duhem_dashboard::export(&reader, out.path()).await.unwrap();
    let exported: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.path().join("api/runs/root.json")).unwrap())
            .unwrap();
    let url = exported["native"][1]["evidence"][0]["url"]
        .as_str()
        .unwrap();
    assert!(url.starts_with("run/leaf/artifact/"));
    assert!(out.path().join(url).is_file());
    let exported: serde_json::Value = serde_json::from_slice(
        &std::fs::read(out.path().join("api/runs/root/failure.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(exported["failing_native"][1]["evidence"][0]["url"], url);
}

#[tokio::test]
async fn native_layers_include_executed_duhem_checks() {
    let (_tmp, rw, ro) = common::open_stores().await;
    seed(rw.clone(), "root", None, Some(false)).await;
    seed(rw, "leaf", Some("root"), None).await;
    let reader = EvidenceReader::new(ro);
    let detail = reader.run_detail("root").await.unwrap().unwrap();
    let duhem = detail
        .layers
        .iter()
        .find(|row| row.layer == "duhem")
        .unwrap();
    assert_eq!(duhem.tests, 2);
    assert_eq!(duhem.passed, 2);
    assert_eq!(duhem.duration_ms, 0);
}

#[tokio::test]
async fn native_absence_preserves_leaf_envelope_bytes() {
    let (_tmp, rw, ro) = common::open_stores().await;
    seed(rw, "leaf", None, None).await;
    let reader = EvidenceReader::new(ro);
    let envelope = reader.failure_envelope("leaf").await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_string(&envelope).unwrap(),
        r#"{"run_id":"leaf","verification":"native","verdict":"pass","failing":[]}"#
    );
}

#[tokio::test]
async fn suite_native_gate_applies_to_detail_and_envelope() {
    for required in [true, false] {
        let (_tmp, rw, ro) = common::open_stores().await;
        seed(rw.clone(), "root", None, Some(required)).await;
        seed(rw, "leaf", Some("root"), None).await;
        let reader = EvidenceReader::new(ro);
        let expected = Some(if required {
            VerdictState::Fail
        } else {
            VerdictState::Pass
        });
        assert_eq!(
            reader.run_detail("root").await.unwrap().unwrap().verdict,
            expected
        );
        assert_eq!(
            reader
                .failure_envelope("root")
                .await
                .unwrap()
                .unwrap()
                .verdict,
            expected
        );
    }
}
