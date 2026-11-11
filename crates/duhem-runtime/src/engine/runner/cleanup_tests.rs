fn cleanup_definition(engine: &mut Engine, yaml: &str) -> VerificationDefinition {
    engine.register_test_action(Box::new(StubAction::new("fake/setup", Outcome::Ok)));
    load_flows(yaml)
}

#[tokio::test]
async fn required_cleanup_preserves_body_and_independent_checks() {
    for body in ["true", "false", "$steps.missing.outputs.value == true"] {
        for policy in ["required", "diagnostic_only"] {
            let (mut engine, tmp) = engine_for_test().await;
            engine.register_test_action(Box::new(StubAction::new("fake/down", Outcome::Error)));
            let definition = cleanup_definition(&mut engine, &format!(r#"
verification: cleanup gate
flows:
  resource:
    steps: [{{uses: fake/setup}}]
    cleanup: [{{id: release, uses: fake/down}}]
    cleanup_policy: {policy}
criteria:
  - id: AC
    description: Preserve body and keep independent checks
    checks:
      - id: body
        setup: [{{call: resource}}]
        steps: [{{id: missing, uses: fake/setup}}]
        assertions: ["{body}"]
      - id: independent
        assertions: ["true"]
"#));
            let outcome = engine.run_with_metadata(&definition, BTreeMap::new()).await.unwrap();
            let body_state = outcome.verdict.criteria[0].state;
            assert_eq!(outcome.verdict.criteria[0].checks[1].state, VerdictState::Pass);
            assert_eq!(outcome.cleanup.len(), 1);
            assert_eq!(outcome.cleanup[0].required, policy == "required");
            let expected = if policy == "required" { VerdictState::Fail } else { body_state };
            assert_eq!(outcome.verdict.state, expected);
            assert_eq!(outcome.body_verdict, (expected != body_state).then_some(body_state));
            let store = SqliteStore::open(tmp.path().join("duhem.db")).await.unwrap();
            let record = duhem_evidence::Store::get_run(&store, &outcome.run_id).await.unwrap().unwrap();
            assert_eq!(record.verdict, Some(expected));
        }
    }
}

#[tokio::test]
async fn flow_cleanup_judgment_is_distinct_from_execution() {
    for execution in [Outcome::Ok, Outcome::Error, Outcome::Timeout] {
        let (mut engine, tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(StubAction::new("fake/judge", execution.clone())
            .judging().with_output("satisfied", serde_json::json!(false))));
        let definition = cleanup_definition(&mut engine, r#"
verification: cleanup judgment
flows:
  resource:
    steps: [{uses: fake/setup}]
    cleanup: [{id: release, uses: fake/judge}]
criteria:
  - id: AC
    description: Flow body and cleanup actions judge independently
    checks:
      - id: body
        setup: [{call: resource}]
        assertions: ["true"]
"#);
        let outcome = engine.run_with_metadata(&definition, BTreeMap::new()).await.unwrap();
        assert_eq!(outcome.verdict.state, VerdictState::Fail);
        assert!(outcome.cleanup[0].required);
        if execution == Outcome::Ok {
            assert_eq!(outcome.body_verdict, Some(VerdictState::Pass));
            let store = SqliteStore::open(tmp.path().join("duhem.db")).await.unwrap();
            let events = duhem_evidence::Store::run_events(&store, &outcome.run_id).await.unwrap();
            assert!(events.iter().any(|event| matches!(event.payload,
                EventPayload::SetupStepFinished {
                    phase: duhem_evidence::StepPhase::Teardown,
                    outcome: duhem_evidence::StepOutcome::Ok,
                    judgment: Some(VerdictState::Fail), ..
                })));
        }
    }
}

#[tokio::test]
async fn ordinary_cleanup_observations_do_not_judge() {
    let (mut engine, _tmp) = engine_for_test().await;
    engine.register_test_action(Box::new(StubAction::new("fake/observe", Outcome::Ok)
        .with_output("exit_code", serde_json::json!(1))
        .with_output("status", serde_json::json!(500))));
    let definition = cleanup_definition(&mut engine, r#"
verification: cleanup observation
flows:
  resource:
    steps: [{uses: fake/observe}]
    cleanup: [{uses: fake/observe}]
criteria:
  - id: AC
    description: Observations remain observations
    checks:
      - id: body
        setup: [{call: resource}]
        assertions: ["true"]
"#);
    let outcome = engine.run_with_metadata(&definition, BTreeMap::new()).await.unwrap();
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    assert!(outcome.cleanup.is_empty());
}

struct CleanupRetryAction {
    name: &'static str,
    cleanup: bool,
    calls: AtomicUsize,
}

#[async_trait]
impl Dispatch for CleanupRetryAction {
    fn uses(&self) -> &'static str { self.name }
    fn requires_page(&self) -> bool { false }
    async fn invoke(&self, _page: Option<&Page>, _index: usize, _with: &serde_yml::Value,
        _env: &BTreeMap<String, String>) -> Result<ActionResult, ActionError> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        Ok(if first && self.cleanup { ActionResult::error() }
            else if first { ActionResult::timeout() } else { ActionResult::ok() })
    }
}

#[tokio::test]
async fn cleanup_failure_from_first_attempt_survives_passing_retry() {
    for policy in ["required", "diagnostic_only"] {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.retry = Some(RetryPolicy { max: 1, backoff: RetryBackoff::Exponential });
        engine.retry_backoff_base = Duration::ZERO;
        for (name, cleanup) in [("fake/body", false), ("fake/down", true)] {
            engine.register_test_action(Box::new(CleanupRetryAction { name, cleanup, calls: AtomicUsize::new(0) }));
        }
        let definition = cleanup_definition(&mut engine, &format!(r#"
verification: retry cleanup gate
flows:
  resource:
    steps: [{{uses: fake/setup}}]
    cleanup: [{{uses: fake/down}}]
    cleanup_policy: {policy}
criteria:
  - id: AC
    description: First attempt cleanup remains evidence
    checks:
      - id: body
        setup: [{{call: resource}}]
        steps: [{{uses: fake/body}}]
        assertions: ["true"]
"#));
        let outcome = engine.run_with_metadata(&definition, BTreeMap::new()).await.unwrap();
        assert_eq!(outcome.verdict.criteria[0].checks[0].state, VerdictState::Pass);
        assert_eq!(outcome.verdict.state, if policy == "required" { VerdictState::Fail } else { VerdictState::Pass });
        assert_eq!(outcome.cleanup.len(), 1);
        assert_eq!(outcome.cleanup[0].required, policy == "required");
    }
}

#[tokio::test]
async fn body_engine_error_drains_flow_cleanup_and_keeps_original_error() {
    let (mut engine, _tmp) = engine_for_test().await;
    let down = StubAction::new("fake/down", Outcome::Error);
    let down_calls = down.invocations.clone();
    engine.register_test_action(Box::new(down));
    engine.register_test_action(Box::new(StubAction::new("fake/body", Outcome::Ok)));
    let teardown = StubAction::new("fake/teardown", Outcome::Ok);
    let teardown_calls = teardown.invocations.clone();
    engine.register_test_action(Box::new(teardown));
    let definition = cleanup_definition(&mut engine, r#"
verification: Drain after engine error
flows:
  resource:
    steps: [{uses: fake/setup}]
    cleanup: [{uses: fake/down}]
criteria:
  - id: AC
    description: Original engine failure survives cleanup
    checks:
      - id: body
        setup: [{call: resource}]
        steps:
          - id: missing
            uses: fake/setup
          - uses: fake/body
            with: {value: $steps.missing.outputs.value}
        teardown: [{uses: fake/teardown}]
        assertions: ["true"]
"#);
    let error = engine.run_with_metadata(&definition, BTreeMap::new()).await.unwrap_err();
    assert!(error.to_string().contains("missing"), "{error}");
    assert_eq!(down_calls.load(Ordering::SeqCst), 1);
    assert_eq!(teardown_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn required_cleanup_dispatch_error_masks_registered_secrets() {
    let (mut engine, _tmp) = engine_for_test().await;
    engine.register_test_action(Box::new(FailingAction {
        uses: "fake/down", field: "token", message: "cleanup could not release",
    }));
    let definition = cleanup_definition(&mut engine, r#"
verification: Mask cleanup failure
inputs:
  token: {type: string, secret: true}
flows:
  resource:
    params: {token: {type: string, secret: true}}
    steps: [{uses: fake/setup}]
    cleanup:
      - uses: fake/down
        with: {token: $params.token}
criteria:
  - id: AC
    description: Cleanup errors are masked at the evidence boundary
    checks:
      - id: body
        setup: [{call: resource, with: {token: $inputs.token}}]
        assertions: ["true"]
"#);
    let secret = "registered-cleanup-secret";
    let outcome = engine.run_with_metadata(&definition,
        BTreeMap::from([("token".into(), serde_json::json!(secret))])).await.unwrap();
    assert_eq!(outcome.verdict.state, VerdictState::Fail);
    assert!(!outcome.cleanup[0].detail.as_ref().unwrap().contains(secret));
    let events = Trace::from_store(engine.store.as_ref().unwrap().as_ref(), &outcome.run_id).await.unwrap().into_events();
    assert!(!serde_json::to_string(&events).unwrap().contains(secret));
}

#[tokio::test]
async fn missing_cleanup_judging_output_blocks_required_gate_and_replays() {
    for policy in ["required", "diagnostic_only"] {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(StubAction::new("fake/judge", Outcome::Ok).judging()));
        let definition = cleanup_definition(&mut engine, &format!(r#"
verification: Missing cleanup judging output
flows:
  resource:
    steps: [{{uses: fake/setup}}]
    cleanup: [{{uses: fake/judge}}]
    cleanup_policy: {policy}
criteria:
  - id: AC
    description: A required cleanup judgment must be observable
    checks:
      - id: body
        setup: [{{call: resource}}]
        assertions: ["true"]
"#));
        let outcome = engine.run_with_metadata(&definition, BTreeMap::new()).await.unwrap();
        assert_eq!(outcome.verdict.state, if policy == "required" { VerdictState::Fail } else { VerdictState::Pass });
        assert_eq!(outcome.cleanup.len(), 1);
        assert_eq!(outcome.cleanup[0].required, policy == "required");
        assert!(outcome.cleanup[0].detail.as_ref().unwrap().contains("satisfied"));
        let trace = Trace::from_store(engine.store.as_ref().unwrap().as_ref(), &outcome.run_id).await.unwrap();
        let replayed = duhem_evidence::replay(&trace).unwrap();
        assert_eq!(replayed.run.state, outcome.verdict.state);
        let mut events = trace.into_events();
        if let EventPayload::RunFinished { body_verdict, .. } = &mut events.last_mut().unwrap().payload {
            *body_verdict = Some(VerdictState::Fail);
        }
        let malformed = Trace::from_events(events).unwrap();
        assert!(matches!(duhem_evidence::replay(&malformed),
            Err(duhem_evidence::ReplayError::Divergence(duhem_evidence::ReplayDivergence::Body { .. }))));
    }
}
