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
    let mut writer = EvidenceWriter::begin_scoped_with_secrets_and_lineage(
        store,
        id,
        "verifications/native.yml",
        BTreeMap::new(),
        RunScope::default(),
        lineage.clone(),
        SecretRegistry::new(),
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
        let result: NativeRunSummary = serde_json::from_value(serde_json::json!({
            "id": "lint", "layer": "static", "outcome": "fail", "duration_ms": 12,
            "required": required, "tests": 1, "exit_code": 1,
        }))
        .unwrap();
        writer
            .append(EventPayload::NativeRunFinished { result })
            .await
            .unwrap();
    }
    writer
        .append(EventPayload::RunFinished {
            body_verdict: None,
            verdict: Some(VerdictState::Pass),
        })
        .await
        .unwrap();
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
