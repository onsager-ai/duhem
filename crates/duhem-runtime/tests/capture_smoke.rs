//! Failure-evidence capture end-to-end (spec #202).
//!
//! Drives real ui checks through a real Playwright browser against an
//! in-process axum fixture and asserts the capture contract: a
//! non-pass ui check records `capture/screenshot` (PNG) +
//! `capture/dom` (HTML) blob observations under the default
//! `on-failure` policy; `always` extends capture to passing checks;
//! `off` records nothing; and a captured trace still replays to the
//! same verdict (the hub's ingest revalidation must not be perturbed
//! by capture observations).
//!
//! `#[ignore]`'d for the same reason as `engine_smoke.rs`: the
//! browser needs `npx playwright install chromium`. Locally:
//! `cargo test -p duhem-runtime --test capture_smoke -- --ignored`.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use duhem_actions::RunBrowser;
use duhem_evidence::{Event, EventPayload, ObservationValue, SqliteStore, Store, Trace, replay};
use duhem_judge::VerdictState;
use duhem_runtime::{CapturePolicy, Engine, RunOutcome};
use duhem_schema::VerificationDefinition;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const STATIC_HTML: &str = r#"<!doctype html>
<html><head><title>capture fixture</title></head>
<body><main><button id="create">Create</button></main></body></html>"#;

/// The worked example from spec #202: the page has no
/// "Sign in with SSO" button, so the assertion fails.
const FAILING_YAML: &str = r#"
verification: capture smoke — failing ui check

inputs:
  fixture_url:
    type: string

criteria:
  - id: AC-1
    description: The page offers SSO sign-in.
    checks:
      - id: AC-1.1
        description: Open the page and observe the SSO button.
        steps:
          - uses: ui/navigate
            with:
              url: $inputs.fixture_url
          - id: sso
            uses: ui/assert-element
            with:
              locator: { role: button, name: Sign in with SSO }
              expected: visible
              within: 1s
            outputs:
              satisfied: satisfied
        assertions:
          - $steps.sso.outputs.satisfied == true
"#;

const PASSING_YAML: &str = r#"
verification: capture smoke — passing ui check

inputs:
  fixture_url:
    type: string

criteria:
  - id: AC-1
    description: The page shows the Create button.
    checks:
      - id: AC-1.1
        description: Open the page and observe the Create button.
        steps:
          - uses: ui/navigate
            with:
              url: $inputs.fixture_url
          - id: create
            uses: ui/assert-element
            with:
              locator: { role: button, name: Create }
              expected: visible
              within: 5s
            outputs:
              satisfied: satisfied
        assertions:
          - $steps.create.outputs.satisfied == true
"#;

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

async fn start_fixture() -> Fixture {
    let app = Router::new().route("/", get(|| async { axum::response::Html(STATIC_HTML) }));
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

/// Run `yaml` under `policy` and hand back the outcome, the full
/// event stream, and the store (for blob fetches).
async fn run_under(
    policy: CapturePolicy,
    yaml: &str,
) -> (RunOutcome, Vec<Event>, Arc<SqliteStore>, tempfile::TempDir) {
    let fx = start_fixture().await;
    let url = format!("http://{}/", fx.addr);
    let def = VerificationDefinition::from_yaml_str(yaml).expect("parse def");

    let browser = RunBrowser::launch(false)
        .await
        .expect("launch chromium (run `npx playwright install chromium`)");

    let tmp = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(
        SqliteStore::open(tmp.path().join("duhem.db"))
            .await
            .expect("open store"),
    );
    let mut engine = Engine::new()
        .with_browser(browser)
        .with_store(store.clone())
        .with_capture(policy);

    let mut inputs = BTreeMap::new();
    inputs.insert("fixture_url".to_string(), serde_json::Value::String(url));

    let outcome = engine
        .run_with_metadata(&def, inputs)
        .await
        .expect("engine run");
    let trace = Trace::from_store(store.as_ref(), &outcome.run_id)
        .await
        .expect("open trace");
    let events = trace.events().to_vec();
    (outcome, events, store, tmp)
}

/// `(output_name, blob_sha256)` for every `capture/*` observation.
fn capture_blobs(events: &[Event]) -> Vec<(String, String)> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::StepObservation {
                output_name,
                value: ObservationValue::Blob { blob_sha256 },
                ..
            } if output_name.starts_with("capture/") => {
                Some((output_name.clone(), blob_sha256.clone()))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn failing_ui_check_captures_screenshot_and_dom_by_default() {
    let (outcome, events, store, _tmp) = run_under(CapturePolicy::default(), FAILING_YAML).await;
    assert_eq!(outcome.verdict.state, VerdictState::Fail);

    let blobs = capture_blobs(&events);
    let names: Vec<&str> = blobs.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["capture/screenshot", "capture/dom"],
        "expected both captures, got {names:?}"
    );

    // The screenshot is a real PNG and the DOM snapshot is the real
    // page — not placeholders.
    let png = store
        .get_blob(&blobs[0].1)
        .await
        .expect("get png")
        .expect("png blob present");
    assert!(
        png.starts_with(&[0x89, b'P', b'N', b'G']),
        "screenshot blob is not a PNG"
    );
    let dom = store
        .get_blob(&blobs[1].1)
        .await
        .expect("get dom")
        .expect("dom blob present");
    let dom = String::from_utf8(dom).expect("dom is utf-8");
    assert!(
        dom.contains("Create"),
        "dom snapshot should carry the fixture page, got: {dom}"
    );

    // The reporter-facing failure carries the same refs.
    assert_eq!(outcome.failures.len(), 1);
    let caps = &outcome.failures[0].captures;
    assert_eq!(caps.len(), 2, "CheckFailure.captures = {caps:?}");
    assert_eq!(caps[0].kind, "capture/screenshot");
    assert_eq!(caps[1].kind, "capture/dom");
    assert_eq!(caps[0].sha256, blobs[0].1);

    // Captured traces still replay to the recorded verdict — the
    // hub's ingest revalidation path must be indifferent to
    // capture observations.
    let trace = Trace::from_store(store.as_ref(), &outcome.run_id)
        .await
        .expect("reopen trace");
    let replayed = replay(&trace).expect("replay");
    assert_eq!(replayed.run.state, VerdictState::Fail);
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn passing_ui_check_captures_nothing_by_default() {
    let (outcome, events, _store, _tmp) = run_under(CapturePolicy::default(), PASSING_YAML).await;
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    assert!(
        capture_blobs(&events).is_empty(),
        "on-failure policy must not capture on pass"
    );
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn always_policy_captures_on_pass() {
    let (outcome, events, _store, _tmp) = run_under(CapturePolicy::Always, PASSING_YAML).await;
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    let names: Vec<String> = capture_blobs(&events).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, vec!["capture/screenshot", "capture/dom"]);
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn off_policy_never_captures() {
    let (outcome, events, _store, _tmp) = run_under(CapturePolicy::Off, FAILING_YAML).await;
    assert_eq!(outcome.verdict.state, VerdictState::Fail);
    assert!(
        capture_blobs(&events).is_empty(),
        "off policy must not capture even on fail"
    );
}

#[test]
fn capture_policy_parses_the_cli_tokens() {
    assert_eq!(
        "on-failure".parse::<CapturePolicy>().unwrap(),
        CapturePolicy::OnFailure
    );
    assert_eq!(
        "always".parse::<CapturePolicy>().unwrap(),
        CapturePolicy::Always
    );
    assert_eq!("off".parse::<CapturePolicy>().unwrap(), CapturePolicy::Off);
    assert!("sometimes".parse::<CapturePolicy>().is_err());
}
