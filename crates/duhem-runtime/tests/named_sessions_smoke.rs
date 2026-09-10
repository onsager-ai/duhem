//! Real HTTP + SQLite permission app through Chromium, including a failing control.
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use duhem_actions::RunBrowser;
use duhem_dashboard::EvidenceReader;
use duhem_evidence::{EventPayload, SqliteStore, Trace};
use duhem_judge::VerdictState;
use duhem_runtime::{CapturePolicy, Engine};
use duhem_schema::VerificationDefinition;

struct App(Child);
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
#[ignore = "requires Playwright Chromium; explicitly run by just test browser-actions"]
async fn permissions_isolation_signed_out_seeded_sibling_and_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let app_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../verifications/named-sessions-example/app.py"
    );
    let mut app = App(Command::new("python3")
        .arg(app_path)
        .args(["--port", "0", "--database"])
        .arg(tmp.path().join("app.sqlite"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap());
    let mut url = String::new();
    BufReader::new(app.0.stdout.take().unwrap())
        .read_line(&mut url)
        .unwrap();
    assert!(url.starts_with("http://127.0.0.1:"), "app readiness: {url}");
    let inputs = BTreeMap::from([("base_url".into(), serde_json::json!(url.trim()))]);
    let mut def = VerificationDefinition::from_yaml_str(include_str!(
        "../../../verifications/named-sessions-example/duhem.yml"
    ))
    .unwrap();
    // Check hooks address the same contexts: setup reaches the login page;
    // teardown observes admin's still-authenticated page after the body.
    let check = &mut def.criteria[0].checks[0];
    check.setup = serde_yml::from_str(
        r#"
- session: admin
  uses: ui/navigate
  with: { url: '$runtime.format("{}/login", $inputs.base_url)' }
"#,
    )
    .unwrap();
    check.teardown = serde_yml::from_str(
        r#"
- session: admin
  uses: ui/assert-url
  with: { matches: /grant }
"#,
    )
    .unwrap();
    // Check cookie isolation above and both DOM storage mechanisms here.
    check.steps.extend(
        serde_yml::from_str::<Vec<duhem_schema::Step>>(
            r#"
- session: user1
  uses: ui/navigate
  with: { url: '$runtime.format("{}/storage", $inputs.base_url)' }
- session: user1
  uses: ui/assert-element
  with: { locator: { role: heading, name: Clean storage }, expected: visible }
"#,
        )
        .unwrap(),
    );
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("evidence.sqlite"))
            .await
            .unwrap(),
    );
    let browser = RunBrowser::launch(false)
        .await
        .expect("launch installed Chromium");
    let mut engine = Engine::new()
        .with_browser(browser)
        .with_store(store.clone())
        .with_capture(CapturePolicy::Always);
    let result = engine
        .run_with_metadata(&def, inputs.clone())
        .await
        .unwrap();
    assert_eq!(result.verdict.state, VerdictState::Pass);
    assert!(
        result.cleanup.is_empty(),
        "named teardown must see admin's existing page: {:?}",
        result.cleanup
    );
    let trace = Trace::from_store(store.as_ref(), &result.run_id)
        .await
        .unwrap();
    let mut expected = Vec::new();
    for criterion in &def.criteria {
        for check in &criterion.checks {
            expected.extend(check.steps.iter().map(|step| step.session.as_deref()));
        }
    }
    let actual: Vec<_> = trace
        .events()
        .iter()
        .filter_map(|event| {
            matches!(event.payload, EventPayload::StepStarted { .. })
                .then_some(event.session.as_deref())
        })
        .collect();
    assert_eq!(
        actual, expected,
        "authored sequential order and selected session"
    );
    for event in trace.events() {
        if matches!(
            event.payload,
            EventPayload::StepObservation { .. } | EventPayload::StepFinished { .. }
        ) {
            assert!(
                event.session.is_some(),
                "step evidence must identify its context: {event:?}"
            );
        }
    }
    let reader = EvidenceReader::new(store.clone());
    let detail = reader
        .check_detail(&result.run_id, "AC-1", "AC-1.1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.sessions.len(), 2);
    assert!(
        detail.replay.is_none(),
        "no single-clock replay for named contexts"
    );
    for name in ["admin", "user1"] {
        let replay = detail
            .sessions
            .iter()
            .find(|s| s.session.as_deref() == Some(name))
            .unwrap();
        assert!(!replay.steps.is_empty());
        assert!(!replay.network.is_empty());
        for step in &replay.steps {
            if let Some(shot) = &step.screenshot {
                assert_eq!(shot.session.as_deref(), Some(name));
            }
            assert_eq!(
                def.criteria[0].checks[0].steps[step.step_index as usize]
                    .session
                    .as_deref(),
                Some(name)
            );
        }
        for kind in [
            "capture/dom",
            "capture/screenshot",
            "capture/network",
            "capture/session-evidence",
        ] {
            assert!(
                detail
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.kind == kind
                        && artifact.session.as_deref() == Some(name)),
                "missing {kind} for {name}"
            );
        }
    }

    // Negative control: same real app, same authored assertions, one cookie jar.
    // Admin's session reaches /reports, so the Login heading assertion MUST fail.
    let mut contaminated = def.clone();
    contaminated.criteria.truncate(1);
    let isolation_step_index = contaminated.criteria[0].checks[0]
        .steps
        .iter()
        .position(|step| step.id.as_deref() == Some("user1_is_signed_out"))
        .expect("fixture must label the signed-out Login heading assertion");
    for step in &mut contaminated.criteria[0].checks[0].steps {
        if step.session.as_deref() == Some("user1") {
            step.session = Some("admin".into());
        }
    }
    let control = engine
        .run_with_metadata(&contaminated, inputs)
        .await
        .unwrap();
    assert_eq!(
        control.verdict.state,
        VerdictState::Fail,
        "isolation test must detect shared cookies"
    );
    let control_trace = Trace::from_store(store.as_ref(), &control.run_id)
        .await
        .unwrap();
    assert!(
        control_trace.events().iter().any(|event| matches!(
            event.payload,
            EventPayload::AssertionEvaluated {
                step_index: Some(index),
                state: VerdictState::Fail,
                ..
            } if index as usize == isolation_step_index
        )),
        "the signed-out Login heading assertion, specifically, must go red"
    );
    // Even an unused context must open before the first (page-free) step.
    // A malformed seed must prevent that step's filesystem side effect.
    let marker = tmp.path().join("must-not-run");
    let eager = VerificationDefinition::from_yaml_str(
        r#"
verification: Eager context allocation
inputs:
  state: { type: object }
  marker: { type: string }
criteria:
  - id: AC-1
    description: Every context opens before actions run.
    checks:
      - id: AC-1.1
        sessions:
          a_signed_out: ~
          z_bad_seed: $inputs.state
        steps:
          - uses: cli/invoke
            with: { command: [touch, $inputs.marker] }
        assertions: ["true"]
"#,
    )
    .unwrap();
    let failed = engine
        .run(
            &eager,
            BTreeMap::from([
                ("state".into(), serde_json::json!({"cookies": "malformed"})),
                ("marker".into(), serde_json::json!(marker)),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(
        failed.state,
        VerdictState::Inconclusive(duhem_judge::InconclusiveCause::EnvironmentError)
    );
    assert!(
        !marker.exists(),
        "unused contexts must be allocated eagerly"
    );
}
