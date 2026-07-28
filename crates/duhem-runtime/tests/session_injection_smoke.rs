//! End-to-end browser-session injection coverage (spec #347).
//!
//! The ignored test drives a real login fixture through Playwright:
//! setup captures ambient state, sibling checks receive fresh seeded
//! contexts, and evidence/export/dashboard projections are inspected
//! for credential absence. Browser-free edge cases remain in the
//! ordinary workspace gate.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use duhem_actions::RunBrowser;
use duhem_dashboard::EvidenceReader;
use duhem_evidence::{EventPayload, RunBundle, SqliteStore, Trace};
use duhem_judge::{InconclusiveCause, VerdictState};
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const CREDENTIAL: &str = "credential-347-browser-session";

const LOGIN_HTML: &str = r#"<!doctype html>
<html><body>
  <h1>Login</h1>
  <button onclick="
    document.cookie = 'session=credential-347-browser-session; path=/';
    localStorage.setItem('baseline', 'clean');
    location.href = '/workspaces';
  ">Sign in</button>
</body></html>"#;

const WORKSPACES_HTML: &str = r#"<!doctype html>
<html><body>
  <h1>Workspaces</h1>
  <button onclick="localStorage.setItem('mutated', 'yes')">Mutate session</button>
</body></html>"#;

const MUTATION_HTML: &str = r#"<!doctype html>
<html><body><script>
  document.body.innerHTML = localStorage.getItem('mutated')
    ? '<h1>Leaked mutation</h1>'
    : '<h1>Clean baseline</h1>';
</script></body></html>"#;

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

async fn authenticated(headers: HeaderMap) -> Response {
    let signed_in = headers
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains(&format!("session={CREDENTIAL}")));
    if signed_in {
        Html(WORKSPACES_HTML).into_response()
    } else {
        Redirect::temporary("/login").into_response()
    }
}

async fn start_fixture() -> Fixture {
    let app = Router::new()
        .route("/login", get(|| async { Html(LOGIN_HTML) }))
        .route("/workspaces", get(authenticated))
        .route("/mutation", get(|| async { Html(MUTATION_HTML) }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Fixture {
        addr,
        _server: server,
    }
}

fn operator_state(addr: SocketAddr) -> serde_json::Value {
    serde_json::json!({
        "cookies": [{
            "name": "session",
            "value": CREDENTIAL,
            "domain": "127.0.0.1",
            "path": "/",
            "expires": -1,
            "httpOnly": false,
            "secure": false,
            "sameSite": "Lax"
        }],
        "origins": [{
            "origin": format!("http://{addr}"),
            "localStorage": [{"name": "baseline", "value": "clean"}]
        }]
    })
}

fn authenticated_vd() -> VerificationDefinition {
    VerificationDefinition::from_yaml_str(
        r#"
verification: browser session injection
inputs:
  base_url: { type: string }
  operator_session: { type: object, secret: true }
setup:
  - id: login_page
    uses: ui/navigate
    with:
      url: $runtime.format("{}/login", $inputs.base_url)
  - id: login
    uses: ui/click
    with: { role: button, name: Sign in }
  - id: session
    uses: ui/capture-session
criteria:
  - id: AC-1
    description: A signed-out visitor is sent to the login page.
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/navigate
            with:
              url: $runtime.format("{}/workspaces", $inputs.base_url)
          - uses: ui/assert-url
            with: { matches: "/login" }
  - id: AC-2
    description: Authenticated checks begin from one baseline without sharing mutations.
    checks:
      - id: AC-2.1
        session: $setup.session.outputs.state
        steps:
          - uses: ui/navigate
            with:
              url: $runtime.format("{}/workspaces", $inputs.base_url)
          - uses: ui/assert-element
            with:
              locator: { role: heading, name: Workspaces }
              expected: visible
          - uses: ui/click
            with: { role: button, name: Mutate session }
      - id: AC-2.2
        session: $setup.session.outputs.state
        steps:
          - uses: ui/navigate
            with:
              url: $runtime.format("{}/mutation", $inputs.base_url)
          - uses: ui/assert-element
            with:
              locator: { role: heading, name: Clean baseline }
              expected: visible
  - id: AC-3
    description: Operator-supplied storage state can seed a check.
    checks:
      - id: AC-3.1
        session: $inputs.operator_session
        steps:
          - uses: ui/navigate
            with:
              url: $runtime.format("{}/workspaces", $inputs.base_url)
          - uses: ui/assert-element
            with:
              locator: { role: heading, name: Workspaces }
              expected: visible
"#,
    )
    .unwrap()
}

fn inputs(fixture: &Fixture) -> BTreeMap<String, serde_json::Value> {
    BTreeMap::from([
        (
            "base_url".into(),
            serde_json::json!(format!("http://{}", fixture.addr)),
        ),
        ("operator_session".into(), operator_state(fixture.addr)),
    ])
}

fn digest_for(trace: &Trace, check: &str) -> String {
    trace
        .events()
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::CheckFinished {
                check_id,
                session_digest,
                ..
            } if check_id == check => session_digest.clone(),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing session digest for {check}"))
}

#[tokio::test]
async fn page_free_session_is_ignored_and_the_check_passes() {
    let def = VerificationDefinition::from_yaml_str(
        r#"
verification: unused session
inputs:
  state: { type: object }
criteria:
  - id: AC-1
    description: A page-free check remains runnable.
    checks:
      - id: AC-1.1
        session: $inputs.state
        steps:
          - uses: cli/invoke
            with: { command: ["true"] }
        assertions: ["true"]
"#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .unwrap(),
    );
    let verdict = Engine::new()
        .with_store(store)
        .run(
            &def,
            BTreeMap::from([(
                "state".into(),
                serde_json::json!({"cookies": [], "origins": []}),
            )]),
        )
        .await
        .unwrap();
    assert_eq!(verdict.state, VerdictState::Pass);
}

#[tokio::test]
async fn missing_browser_with_session_is_environment_error() {
    let def = VerificationDefinition::from_yaml_str(
        r#"
verification: browser unavailable
inputs:
  state: { type: object, secret: true }
criteria:
  - id: AC-1
    description: A browser-backed check cannot pass without a browser.
    checks:
      - id: AC-1.1
        session: $inputs.state
        steps:
          - uses: ui/assert-url
            with: { matches: example }
"#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .unwrap(),
    );
    let verdict = Engine::new()
        .with_store(store)
        .run(
            &def,
            BTreeMap::from([(
                "state".into(),
                serde_json::json!({"cookies": [], "origins": []}),
            )]),
        )
        .await
        .unwrap();
    assert_eq!(
        verdict.state,
        VerdictState::Inconclusive(InconclusiveCause::EnvironmentError)
    );
}

#[tokio::test]
#[ignore = "requires Playwright Chromium; run with just test browser-actions"]
async fn real_login_state_is_captured_seeded_isolated_and_redacted() {
    let fixture = start_fixture().await;
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .unwrap(),
    );
    let browser = RunBrowser::launch(false)
        .await
        .expect("launch chromium (run `duhem browser install`)");
    let mut engine = Engine::new()
        .with_browser(browser)
        .with_store(store.clone());
    let definition = authenticated_vd();

    let first = engine
        .run_with_metadata(&definition, inputs(&fixture))
        .await
        .unwrap();
    assert_eq!(first.verdict.state, VerdictState::Pass);
    let second = engine
        .run_with_metadata(&definition, inputs(&fixture))
        .await
        .unwrap();
    assert_eq!(second.verdict.state, VerdictState::Pass);

    let first_trace = Trace::from_store(store.as_ref(), &first.run_id)
        .await
        .unwrap();
    let second_trace = Trace::from_store(store.as_ref(), &second.run_id)
        .await
        .unwrap();

    // Both sibling checks copied one setup baseline, yet AC-2.2 saw
    // none of AC-2.1's localStorage mutation.
    assert_eq!(
        digest_for(&first_trace, "AC-2.1"),
        digest_for(&first_trace, "AC-2.2")
    );
    // A fixed operator-supplied state hashes identically across runs.
    assert_eq!(
        digest_for(&first_trace, "AC-3.1"),
        digest_for(&second_trace, "AC-3.1")
    );
    assert_eq!(
        digest_for(&first_trace, "AC-2.1"),
        digest_for(&second_trace, "AC-2.1")
    );

    let check_event = first_trace
        .events()
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::CheckFinished {
                check_id,
                session_source,
                session_digest,
                ..
            } if check_id == "AC-2.1" => Some((session_source, session_digest)),
            _ => None,
        });
    assert_eq!(
        check_event,
        Some((
            &Some("$setup.session.outputs.state".into()),
            &Some(digest_for(&first_trace, "AC-2.1"))
        ))
    );

    let trace_json = serde_json::to_string(first_trace.events()).unwrap();
    assert!(!trace_json.contains(CREDENTIAL));

    let bundle = RunBundle::from_store(store.as_ref(), &first.run_id)
        .await
        .unwrap();
    let bundle_json = String::from_utf8(bundle.wire_bytes().unwrap()).unwrap();
    assert!(!bundle_json.contains(CREDENTIAL));

    let dashboard = EvidenceReader::new(store.clone())
        .check_detail(&first.run_id, "AC-2", "AC-2.1")
        .await
        .unwrap()
        .unwrap();
    let dashboard_json = serde_json::to_string(&dashboard).unwrap();
    assert!(!dashboard_json.contains(CREDENTIAL));
    assert!(dashboard_json.contains("session_digest"));

    // Playwright rejects a malformed storageState at context creation;
    // the runtime maps that failure to the browser-environment path.
    let malformed = VerificationDefinition::from_yaml_str(
        r#"
verification: malformed state
inputs:
  state: { type: object, secret: true }
criteria:
  - id: AC-1
    description: Malformed browser state cannot be used.
    checks:
      - id: AC-1.1
        session: $inputs.state
        steps:
          - uses: ui/assert-url
            with: { matches: example }
"#,
    )
    .unwrap();
    let malformed_verdict = engine
        .run(
            &malformed,
            BTreeMap::from([(
                "state".into(),
                serde_json::json!({"cookies": "not-an-array"}),
            )]),
        )
        .await
        .unwrap();
    assert_eq!(
        malformed_verdict.state,
        VerdictState::Inconclusive(InconclusiveCause::EnvironmentError)
    );

    let unresolved = VerificationDefinition::from_yaml_str(
        r#"
verification: unresolved state
criteria:
  - id: AC-1
    description: Missing acquired state cannot be used.
    checks:
      - id: AC-1.1
        session: $setup.nope.outputs.state
        steps:
          - uses: ui/assert-url
            with: { matches: example }
"#,
    )
    .unwrap();
    let unresolved_verdict = engine.run(&unresolved, BTreeMap::new()).await.unwrap();
    assert_eq!(
        unresolved_verdict.state,
        VerdictState::Inconclusive(InconclusiveCause::EnvironmentError)
    );
}
