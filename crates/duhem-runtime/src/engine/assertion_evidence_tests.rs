//! Explicit assertion evidence follows the evaluation that produced the verdict.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use super::{
    context::{RunContext, RunState},
    outcome::evaluate_explicit_assertions,
};
use crate::eval::Value;
use duhem_evidence::{EventPayload, EvidenceWriter, RunScope, SecretRegistry, SqliteStore, Store};
use serde_json::json;

#[tokio::test]
async fn null_missing_short_circuit_and_secrets_survive_the_evidence_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        SqliteStore::open(temp.path().join("evidence.db"))
            .await
            .unwrap(),
    );
    let secret = "assertion-secret-574-unique";
    let mut secrets = SecretRegistry::new();
    secrets.register("token", secret);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut writer = EvidenceWriter::begin_scoped_with_secrets(
        store.clone(),
        "assertion-evidence",
        "case.yml",
        BTreeMap::new(),
        RunScope::default(),
        secrets,
    )
    .await
    .unwrap()
    .with_tee(tx);
    let state = RunState::new(BTreeMap::from([
        ("before".into(), Value::Null),
        ("after".into(), Value::Str("System prompt 4.2k".into())),
        ("token".into(), Value::Str(secret.into())),
    ]));
    let ctx = RunContext::new(&state);
    let check: duhem_schema::Check = serde_yml::from_str(
        r#"
id: C
assertions:
  - equal: [$inputs.before, $inputs.after]
  - "$inputs.missing == 1"
  - "false && $inputs.after == 'unused'"
  - "$inputs.token == 'public'"
"#,
    )
    .unwrap();
    let mut outcomes = Vec::new();
    let mut failures = Vec::new();
    evaluate_explicit_assertions(
        &mut writer,
        &check,
        &ctx,
        false,
        None,
        false,
        false,
        &[],
        &BTreeSet::new(),
        &mut outcomes,
        &mut failures,
    )
    .await
    .unwrap();
    let events = store.run_events("assertion-evidence").await.unwrap();
    let values: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::AssertionEvaluated { operands, .. } => operands.as_ref(),
            _ => None,
        })
        .collect();
    assert_eq!(values.len(), 4);
    assert_eq!(values[0]["$inputs.before"], json!(null));
    assert_eq!(values[0]["$inputs.after"], json!("System prompt 4.2k"));
    assert!(!values[1].contains_key("$inputs.missing"));
    assert!(
        values[2].is_empty(),
        "short circuit must not invent observations"
    );
    let stored = serde_json::to_string(&events).unwrap();
    assert!(!stored.contains(secret), "operand values must be masked");
    assert_ne!(values[3]["$inputs.token"], json!(secret));
    let mut live = Vec::new();
    while let Ok(event) = rx.try_recv() {
        live.push(event);
    }
    assert!(!serde_json::to_string(&live).unwrap().contains(secret));
    assert_eq!(outcomes.len(), 4);
}
