#[tokio::test]
async fn required_cleanup_preserves_body_and_independent_checks() {
    for body in ["true", "false", "$steps.missing.outputs.value == true"] {
        for policy in ["required", "diagnostic_only"] {
            let (mut engine, tmp) = engine_for_test().await;
            engine.register_test_action(Box::new(StubAction::new("fake/down", Outcome::Error)));
            let definition = def(&format!(r#"
verification: cleanup gate
fixtures:
  resource:
    cleanup_policy: {policy}
    up: []
    down: [{{id: release, uses: fake/down}}]
criteria:
  - id: AC
    description: Preserve body and keep independent checks
    checks:
      - id: body
        needs: [resource]
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
async fn fixture_down_judgment_is_distinct_from_execution() {
    for execution in [Outcome::Ok, Outcome::Error, Outcome::Timeout] {
        let (mut engine, tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(StubAction::new("fake/judge", execution.clone())
            .judging().with_output("satisfied", serde_json::json!(false))));
        let definition = def(r#"
verification: cleanup judgment
fixtures:
  resource:
    up: [{uses: fake/judge}]
    down: [{id: release, uses: fake/judge}]
criteria:
  - id: AC
    description: Cleanup judges but fixture up does not
    checks: [{id: body, needs: [resource], assertions: ["true"]}]
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
    let definition = def(r#"
verification: cleanup observation
fixtures:
  resource:
    up: []
    down: [{uses: fake/observe}]
criteria:
  - id: AC
    description: Observations remain observations
    checks: [{id: body, needs: [resource], assertions: ["true"]}]
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
        let definition = def(&format!(r#"
verification: retry cleanup gate
fixtures:
  resource:
    cleanup_policy: {policy}
    up: []
    down: [{{uses: fake/down}}]
criteria:
  - id: AC
    description: First attempt cleanup remains evidence
    checks:
      - id: body
        needs: [resource]
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
