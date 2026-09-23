use super::*;
use sha2::{Digest, Sha256};

#[tokio::test]
#[ignore = "requires Playwright Chromium; run with just test browser-actions"]
async fn lifecycle_seed_cascade_and_fresh_named_contexts() {
    let fixture = start_fixture().await;
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .unwrap(),
    );
    let browser = RunBrowser::launch(false).await.unwrap();
    let mut engine = Engine::new()
        .with_store(store.clone())
        .with_browser(browser);
    let state = operator_state(fixture.addr);
    let expected_digest = hex::encode(Sha256::digest(serde_json::to_vec(&state).unwrap()));
    for (leaf, criterion, check, expected) in [
        (
            "session: $inputs.operator_session",
            "",
            "",
            Some(expected_digest.as_str()),
        ),
        ("session: $inputs.operator_session", "session: ~", "", None),
        (
            "session: ~",
            "session: $inputs.operator_session",
            "",
            Some(expected_digest.as_str()),
        ),
        ("session: $inputs.operator_session", "", "session: ~", None),
        (
            "session: ~",
            "session: ~",
            "session: $inputs.operator_session",
            Some(expected_digest.as_str()),
        ),
    ] {
        let yaml = format!(
            r#"
verification: Cascade
{leaf}
inputs:
  operator_session: {{ type: object, secret: true }}
setup: [{{ uses: ui/capture-session }}]
teardown: [{{ uses: ui/capture-session }}]
fixtures:
  resource:
    up: [{{ uses: ui/capture-session }}]
    down: [{{ uses: ui/capture-session }}]
criteria:
  - id: AC-1
    description: Every scope uses its nearest seed.
    {criterion}
    setup: [{{ uses: ui/capture-session }}]
    teardown: [{{ uses: ui/capture-session }}]
    checks:
      - id: AC-1.1
        {check}
        needs: [resource]
        setup: [{{ uses: ui/capture-session }}]
        teardown: [{{ uses: ui/capture-session }}]
        steps: [{{ uses: ui/capture-session }}]
        assertions: ['true']
"#
        );
        let definition = VerificationDefinition::from_yaml_str(&yaml).unwrap();
        let result = engine
            .run_with_metadata(
                &definition,
                BTreeMap::from([("operator_session".into(), state.clone())]),
            )
            .await
            .unwrap();
        assert_eq!(result.verdict.state, VerdictState::Pass);
        assert!(result.cleanup.is_empty());
        let trace = Trace::from_store(store.as_ref(), &result.run_id)
            .await
            .unwrap();
        let blocks: Vec<_> = trace
            .events()
            .iter()
            .filter(|event| {
                matches!(
                    &event.payload,
                    EventPayload::SetupStarted {
                        check_id: Some(_),
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(blocks.len(), 4, "check setup/teardown and fixture up/down");
        for block in blocks {
            assert_eq!(
                block
                    .session_digest
                    .as_ref()
                    .and_then(serde_json::Value::as_str),
                expected
            );
            let encoded = serde_json::to_string(block).unwrap();
            let decoded: duhem_evidence::Event = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, *block);
        }
        let leaf_digest = leaf.contains("$inputs").then_some(expected_digest.as_str());
        let criterion_digest = if criterion.is_empty() {
            leaf_digest
        } else {
            criterion
                .contains("$inputs")
                .then_some(expected_digest.as_str())
        };
        for event in trace.events() {
            if let EventPayload::SetupStarted {
                check_id: None,
                criterion_id,
                ..
            } = &event.payload
            {
                let expected = if criterion_id.is_some() {
                    criterion_digest
                } else {
                    leaf_digest
                };
                assert_eq!(
                    event
                        .session_digest
                        .as_ref()
                        .and_then(serde_json::Value::as_str),
                    expected
                );
            }
        }
        let body_digest = trace
            .events()
            .iter()
            .find_map(|event| match &event.payload {
                EventPayload::CheckFinished { session_digest, .. } => {
                    Some(session_digest.as_deref())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(body_digest, expected);
    }

    let yaml = r#"
verification: Named lifecycle isolation
inputs:
  base_url: { type: string }
  operator_session: { type: object, secret: true }
  expected_cookie: { type: string, secret: true }
fixtures:
  resource:
    up: [{ uses: ui/capture-session, session: actor }]
    down: [{ uses: ui/capture-session, session: actor }]
criteria:
  - id: AC-1
    description: Cleanup starts from the seed even after the body changes its cookie.
    checks:
      - id: AC-1.1
        sessions: { actor: $inputs.operator_session, other: ~ }
        needs: [resource]
        setup:
          - uses: ui/capture-session
            session: actor
        steps:
          - uses: ui/navigate
            session: actor
            with: { url: '$runtime.format("{}/workspaces", $inputs.base_url)' }
          - uses: ui/click
            session: actor
            with: { role: button, name: Mutate session }
          - id: changed
            uses: ui/capture-session
            session: actor
        assertions:
          - $steps.changed.outputs.state.cookies[0].value == 'body-mutated'
        teardown:
          - id: cleanup_state
            uses: ui/capture-session
            session: actor
          - uses: cli/invoke
            if: $setup.cleanup_state.outputs.state.cookies[0].value != $inputs.expected_cookie
            with: { command: [sh, -c, 'exit 1'] }
"#;
    let mut definition = VerificationDefinition::from_yaml_str(yaml).unwrap();
    definition.max_sessions = Some(2);
    let mut values = inputs(&fixture);
    values.insert("expected_cookie".into(), serde_json::json!(CREDENTIAL));
    let result = engine.run_with_metadata(&definition, values).await.unwrap();
    assert_eq!(result.verdict.state, VerdictState::Pass);
    assert!(result.cleanup.is_empty(), "{:?}", result.cleanup);
    let trace = Trace::from_store(store.as_ref(), &result.run_id)
        .await
        .unwrap();
    let blocks: Vec<_> = trace
        .events()
        .iter()
        .filter(|event| matches!(event.payload, EventPayload::SetupStarted { .. }))
        .collect();
    assert_eq!(blocks.len(), 4);
    for block in blocks {
        assert_eq!(
            block.session_digest.as_ref().unwrap()["actor"],
            expected_digest
        );
        assert!(block.session_digest.as_ref().unwrap()["other"].is_null());
    }
    // Bypass loader validation to exercise the actual allocation guard.
    definition.max_sessions = Some(1);
    let mut values = inputs(&fixture);
    values.insert("expected_cookie".into(), serde_json::json!(CREDENTIAL));
    let error = engine
        .run(&definition, values)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("AC-1.1") && error.contains("max_sessions (1)"),
        "{error}"
    );
}

#[tokio::test]
async fn browser_free_named_scope_does_not_resolve_or_allocate_seed_contexts() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("ran");
    let definition = VerificationDefinition::from_yaml_str(
        r#"
verification: Browser-free named check
inputs:
  state: { type: object }
  marker: { type: string }
criteria:
  - id: AC-1
    description: Browser-free scopes remain independent of browser availability.
    checks:
      - id: AC-1.1
        sessions: { unused: $inputs.state }
        setup:
          - uses: cli/invoke
            with: { command: [touch, $inputs.marker] }
        assertions: ['true']
"#,
    )
    .unwrap();
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .unwrap(),
    );
    let result = Engine::new()
        .with_store(store.clone())
        .run_with_metadata(
            &definition,
            BTreeMap::from([
                ("state".into(), serde_json::json!({"cookies": "malformed"})),
                ("marker".into(), serde_json::json!(marker)),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(result.verdict.state, VerdictState::Pass);
    assert!(marker.exists());
    let trace = Trace::from_store(store.as_ref(), &result.run_id)
        .await
        .unwrap();
    assert!(
        trace
            .events()
            .iter()
            .all(|event| event.session_source.is_none() && event.session_digest.is_none())
    );
}
