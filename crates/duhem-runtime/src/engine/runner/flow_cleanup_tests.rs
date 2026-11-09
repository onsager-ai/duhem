// Flow `cleanup:` registration and unwind (#615).

/// Records `<uses>:<tag>` for every invocation and echoes the tag as the
/// `value` output, so cleanups can be shown to read their call's outputs.
struct TagAction {
    uses: &'static str,
    log: Arc<std::sync::Mutex<Vec<String>>>,
    fail: bool,
}

#[async_trait]
impl Dispatch for TagAction {
    fn uses(&self) -> &'static str {
        self.uses
    }
    fn requires_page(&self) -> bool {
        false
    }
    async fn invoke(
        &self,
        _page: Option<&Page>,
        _index: usize,
        with: &serde_yml::Value,
        _env: &BTreeMap<String, String>,
    ) -> Result<ActionResult, ActionError> {
        let tag = with
            .get("tag")
            .and_then(|tag| tag.as_str())
            .unwrap_or("?")
            .to_string();
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:{tag}", self.uses.trim_start_matches("fake/")));
        let result = if self.fail {
            ActionResult::error()
        } else {
            ActionResult::ok()
        };
        Ok(result.with_output("value", serde_json::json!(tag)))
    }
}

fn tag_engine(
    engine: &mut Engine,
    names: &[(&'static str, bool)],
) -> Arc<std::sync::Mutex<Vec<String>>> {
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    for (uses, fail) in names {
        engine.register_test_action(Box::new(TagAction {
            uses,
            log: log.clone(),
            fail: *fail,
        }));
    }
    log
}

/// Parse and flow-expand a leaf, as `duhem run` does before the engine sees it.
fn load_flows(yaml: &str) -> VerificationDefinition {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("leaf.yml");
    std::fs::write(&path, yaml).unwrap();
    let duhem_schema::Loaded::Leaf { definition, .. } =
        duhem_schema::load(&path).expect("load and expand")
    else {
        panic!("expected a leaf");
    };
    definition
}

const RES_FLOW: &str = r#"
flows:
  res:
    params: {name: {type: string}}
    steps: [{id: make, uses: fake/body, with: {tag: $params.name}}]
    cleanup: [{uses: fake/clean, with: {tag: $steps.make.outputs.value}}]
"#;

#[tokio::test]
async fn check_step_calls_unwind_lifo_after_the_body_and_before_teardown() {
    let (mut engine, _tmp) = engine_for_test().await;
    let log = tag_engine(
        &mut engine,
        &[
            ("fake/body", false),
            ("fake/clean", false),
            ("fake/td", false),
        ],
    );
    let definition = load_flows(&format!(
        r#"
verification: flow cleanup order
{RES_FLOW}
criteria:
  - id: AC
    description: Two calls unwind last first
    checks:
      - id: c
        steps:
          - {{id: one, call: res, with: {{name: a}}}}
          - {{id: two, call: res, with: {{name: b}}}}
          - {{id: last, uses: fake/body, with: {{tag: z}}}}
        teardown: [{{uses: fake/td, with: {{tag: t}}}}]
        assertions: ["true"]
"#
    ));
    let outcome = engine
        .run_with_metadata(&definition, BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    assert!(outcome.cleanup.is_empty());
    assert_eq!(
        *log.lock().unwrap(),
        ["body:a", "body:b", "body:z", "clean:b", "clean:a", "td:t"]
    );
}

#[tokio::test]
async fn a_partial_body_still_cleans_up_and_a_gated_call_registers_nothing() {
    let (mut engine, _tmp) = engine_for_test().await;
    let log = tag_engine(
        &mut engine,
        &[
            ("fake/body", false),
            ("fake/clean", false),
            ("fake/fail", true),
        ],
    );
    let definition = load_flows(
        r#"
verification: partial body
flows:
  part:
    steps:
      - {id: first, uses: fake/body, with: {tag: p1}}
      - {id: second, uses: fake/fail, with: {tag: p2}}
      - {id: third, uses: fake/body, with: {tag: p3}}
    cleanup: [{uses: fake/clean, with: {tag: $steps.first.outputs.value}}]
  res:
    params: {name: {type: string}}
    steps: [{id: make, uses: fake/body, with: {tag: $params.name}}]
    cleanup: [{uses: fake/clean, with: {tag: $steps.make.outputs.value}}]
criteria:
  - id: AC
    description: Cleanup follows entry, not completion
    checks:
      - id: c
        steps:
          - {id: probe, uses: fake/body, with: {tag: yes}}
          - {id: gated, call: res, if: "$steps.probe.outputs.value == 'no'", with: {name: never}}
          - {id: broken, call: part}
        assertions: ["true"]
"#,
    );
    engine
        .run_with_metadata(&definition, BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        *log.lock().unwrap(),
        ["body:yes", "body:p1", "fail:p2", "clean:p1"]
    );
}

#[tokio::test]
async fn required_cleanup_failure_gates_the_run_and_diagnostic_only_does_not() {
    for policy in ["required", "diagnostic_only"] {
        let (mut engine, _tmp) = engine_for_test().await;
        engine.register_test_action(Box::new(StubAction::new("fake/body", Outcome::Ok)));
        engine.register_test_action(Box::new(StubAction::new("fake/down", Outcome::Error)));
        let definition = load_flows(&format!(
            r#"
verification: flow cleanup gate
flows:
  res:
    steps: [{{id: make, uses: fake/body}}]
    cleanup: [{{id: release, uses: fake/down}}]
    cleanup_policy: {policy}
criteria:
  - id: AC
    description: Body passes, cleanup fails
    checks:
      - id: c
        steps: [{{id: tmp, call: res}}]
        assertions: ["true"]
      - id: independent
        assertions: ["true"]
"#
        ));
        let outcome = engine
            .run_with_metadata(&definition, BTreeMap::new())
            .await
            .unwrap();
        let required = policy == "required";
        assert_eq!(
            outcome.verdict.criteria[0].checks[0].state,
            VerdictState::Pass
        );
        assert_eq!(
            outcome.verdict.criteria[0].checks[1].state,
            VerdictState::Pass
        );
        assert_eq!(
            outcome.verdict.state,
            if required {
                VerdictState::Fail
            } else {
                VerdictState::Pass
            }
        );
        assert_eq!(outcome.body_verdict, required.then_some(VerdictState::Pass));
        assert_eq!(outcome.cleanup.len(), 1);
        assert_eq!(outcome.cleanup[0].required, required);
        assert!(
            outcome.cleanup[0]
                .step
                .contains("flow `res` cleanup (call `tmp`)"),
            "{:?}",
            outcome.cleanup[0]
        );
        let trace = Trace::from_store(engine.store.as_ref().unwrap().as_ref(), &outcome.run_id)
            .await
            .unwrap();
        let replayed = duhem_evidence::replay(&trace).unwrap();
        assert_eq!(replayed.run.state, outcome.verdict.state);
        let policy_seen = trace
            .into_events()
            .into_iter()
            .find_map(|event| match event.payload {
                EventPayload::SetupStarted {
                    cleanup_policy,
                    registered_by: Some(by),
                    check_id,
                    criterion_id,
                    ..
                } => {
                    assert_eq!((by.flow.as_str(), by.call.as_str()), ("res", "tmp"));
                    assert_eq!(
                        (criterion_id.as_deref(), check_id.as_deref()),
                        (Some("AC"), Some("c"))
                    );
                    cleanup_policy
                }
                _ => None,
            });
        assert_eq!(policy_seen.as_deref(), Some(policy));
    }
}

#[tokio::test]
async fn cleanup_runs_every_attempt_and_an_earlier_required_failure_is_retained() {
    let (mut engine, _tmp) = engine_for_test().await;
    engine.retry = Some(RetryPolicy {
        max: 1,
        backoff: RetryBackoff::Exponential,
    });
    engine.retry_backoff_base = Duration::ZERO;
    for (name, cleanup) in [("fake/body", false), ("fake/down", true)] {
        engine.register_test_action(Box::new(CleanupRetryAction {
            name,
            cleanup,
            calls: AtomicUsize::new(0),
        }));
    }
    let definition = load_flows(
        r#"
verification: retry flow cleanup
flows:
  res:
    steps: [{id: make, uses: fake/body}]
    cleanup: [{uses: fake/down}]
criteria:
  - id: AC
    description: Each attempt registers its own cleanup
    checks:
      - id: c
        steps: [{id: tmp, call: res}]
        assertions: ["true"]
"#,
    );
    let outcome = engine
        .run_with_metadata(&definition, BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        outcome.verdict.criteria[0].checks[0].state,
        VerdictState::Pass
    );
    assert_eq!(outcome.verdict.state, VerdictState::Fail);
    assert_eq!(outcome.cleanup.len(), 1);
    assert!(outcome.cleanup[0].required);
    let trace = Trace::from_store(engine.store.as_ref().unwrap().as_ref(), &outcome.run_id)
        .await
        .unwrap();
    let blocks = trace
        .into_events()
        .into_iter()
        .filter(|event| {
            matches!(
                event.payload,
                EventPayload::SetupStarted {
                    registered_by: Some(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(blocks, 2);
}

#[tokio::test]
async fn criterion_and_leaf_setup_calls_unwind_at_their_scope_exit() {
    let (mut engine, _tmp) = engine_for_test().await;
    let log = tag_engine(
        &mut engine,
        &[
            ("fake/body", false),
            ("fake/clean", false),
            ("fake/tdc", false),
            ("fake/tdl", false),
        ],
    );
    let definition = load_flows(&format!(
        r#"
verification: scoped flow cleanup
{RES_FLOW}
setup: [{{id: s, call: res, with: {{name: leaf}}}}]
teardown: [{{uses: fake/tdl, with: {{tag: leaf}}}}]
criteria:
  - id: AC
    description: Each scope unwinds its own registrations
    setup: [{{id: cs, call: res, with: {{name: crit}}}}]
    teardown: [{{uses: fake/tdc, with: {{tag: crit}}}}]
    checks:
      - id: c
        steps: [{{uses: fake/body, with: {{tag: z}}}}]
        assertions: ["true"]
"#
    ));
    let outcome = engine
        .run_with_metadata(&definition, BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    assert_eq!(
        *log.lock().unwrap(),
        [
            "body:leaf",
            "body:crit",
            "body:z",
            "clean:crit",
            "tdc:crit",
            "clean:leaf",
            "tdl:leaf",
        ]
    );
}

#[tokio::test]
async fn check_setup_calls_unwind_with_the_check_attempt() {
    let (mut engine, _tmp) = engine_for_test().await;
    let log = tag_engine(
        &mut engine,
        &[
            ("fake/body", false),
            ("fake/clean", false),
            ("fake/td", false),
        ],
    );
    let definition = load_flows(&format!(
        r#"
verification: check setup flow cleanup
{RES_FLOW}
criteria:
  - id: AC
    description: A check setup call unwinds before check teardown
    checks:
      - id: c
        setup: [{{id: pre, call: res, with: {{name: s}}}}]
        steps: [{{uses: fake/body, with: {{tag: z}}}}]
        teardown: [{{uses: fake/td, with: {{tag: t}}}}]
        assertions: ["true"]
"#
    ));
    engine
        .run_with_metadata(&definition, BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        *log.lock().unwrap(),
        ["body:s", "body:z", "clean:s", "td:t"]
    );
}

#[tokio::test]
async fn each_for_each_iteration_registers_its_own_cleanup_with_its_element() {
    let (mut engine, _tmp) = engine_for_test().await;
    let log = tag_engine(&mut engine, &[("fake/body", false), ("fake/clean", false)]);
    engine.register_test_action(Box::new(
        StubAction::new("fake/rows", Outcome::Ok)
            .with_output("items", serde_json::json!(["x", "y"])),
    ));
    let definition = load_flows(&format!(
        r#"
verification: loop flow cleanup
{RES_FLOW}
setup:
  - {{id: rows, uses: fake/rows}}
  - {{for_each: $setup.rows.outputs.items, max: 3, as: item, call: res, with: {{name: $item}}}}
criteria:
  - id: AC
    description: Loop calls register once per iteration
    checks:
      - id: c
        steps:
          - {{id: rows, uses: fake/rows}}
          - {{for_each: $steps.rows.outputs.items, max: 3, as: item, call: res, with: {{name: $item}}}}
        assertions: ["true"]
"#
    ));
    engine
        .run_with_metadata(&definition, BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        *log.lock().unwrap(),
        [
            "body:x", "body:y", "body:x", "body:y", "clean:y", "clean:x", "clean:y", "clean:x",
        ]
    );
}

#[tokio::test]
async fn flow_cleanups_and_fixture_downs_unwind_in_one_lifo_order() {
    let (mut engine, _tmp) = engine_for_test().await;
    let log = tag_engine(
        &mut engine,
        &[
            ("fake/body", false),
            ("fake/clean", false),
            ("fake/td", false),
        ],
    );
    let definition = load_flows(&format!(
        r#"
verification: flow cleanup and fixtures
{RES_FLOW}
fixtures:
  f:
    up: [{{uses: fake/body, with: {{tag: fu}}}}]
    down: [{{uses: fake/clean, with: {{tag: fd}}}}]
criteria:
  - id: AC
    description: One stack orders flow cleanups and fixture downs
    checks:
      - id: c
        needs: [f]
        setup: [{{id: pre, call: res, with: {{name: s}}}}]
        steps: [{{id: a, call: res, with: {{name: a}}}}]
        teardown: [{{uses: fake/td, with: {{tag: t}}}}]
        assertions: ["true"]
"#
    ));
    let outcome = engine
        .run_with_metadata(&definition, BTreeMap::new())
        .await
        .unwrap();
    assert!(outcome.cleanup.is_empty(), "{:?}", outcome.cleanup);
    assert_eq!(
        *log.lock().unwrap(),
        [
            "body:s", "body:fu", "body:a", "clean:a", "clean:fd", "clean:s", "td:t"
        ]
    );
}

const FAN_FLOW: &str = r#"
flows:
  fan:
    params: {items: {type: string}}
    steps:
      - {for_each: $params.items, max: 3, as: n, uses: fake/body, with: {tag: $n}}
    cleanup:
      - {for_each: $params.items, max: 3, as: n, uses: fake/clean, with: {tag: $n}}
"#;

#[tokio::test]
async fn a_flow_that_is_a_loop_registers_once_and_its_cleanup_may_loop() {
    let (mut engine, _tmp) = engine_for_test().await;
    let log = tag_engine(&mut engine, &[("fake/body", false), ("fake/clean", false)]);
    engine.register_test_action(Box::new(
        StubAction::new("fake/rows", Outcome::Ok)
            .with_output("items", serde_json::json!(["x", "y"])),
    ));
    let definition = load_flows(&format!(
        r#"
verification: loop flow
{FAN_FLOW}
setup:
  - {{id: rows, uses: fake/rows}}
  - {{id: fanout, call: fan, with: {{items: $setup.rows.outputs.items}}}}
criteria:
  - id: AC
    description: Loops in the body and in the cleanup
    checks:
      - id: c
        steps:
          - {{id: rows, uses: fake/rows}}
          - {{call: fan, with: {{items: $steps.rows.outputs.items}}}}
        assertions: ["true"]
"#
    ));
    let outcome = engine
        .run_with_metadata(&definition, BTreeMap::new())
        .await
        .unwrap();
    assert!(outcome.cleanup.is_empty(), "{:?}", outcome.cleanup);
    // Once per call, not once per iteration: each cleanup loop runs once.
    assert_eq!(
        *log.lock().unwrap(),
        [
            "body:x", "body:y", "body:x", "body:y", "clean:x", "clean:y", "clean:x", "clean:y",
        ]
    );
}

#[tokio::test]
async fn a_flow_whose_loop_dispatches_nothing_was_not_entered() {
    let (mut engine, _tmp) = engine_for_test().await;
    let log = tag_engine(&mut engine, &[("fake/body", false), ("fake/clean", false)]);
    engine.register_test_action(Box::new(
        StubAction::new("fake/rows", Outcome::Ok).with_output("items", serde_json::json!([])),
    ));
    let definition = load_flows(&format!(
        r#"
verification: empty loop flow
{FAN_FLOW}
setup:
  - {{id: rows, uses: fake/rows}}
  - {{call: fan, with: {{items: $setup.rows.outputs.items}}}}
criteria:
  - id: AC
    description: An empty loop registers no cleanup
    checks:
      - id: c
        steps:
          - {{id: rows, uses: fake/rows}}
          - {{call: fan, with: {{items: $steps.rows.outputs.items}}}}
        assertions: ["true"]
"#
    ));
    let outcome = engine
        .run_with_metadata(&definition, BTreeMap::new())
        .await
        .unwrap();
    assert!(outcome.cleanup.is_empty(), "{:?}", outcome.cleanup);
    assert!(log.lock().unwrap().is_empty());
}
