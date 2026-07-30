//! Failure and replay-evidence capture end-to-end (specs #202 and #337).
//!
//! Drives real ui checks through a real Playwright browser against an
//! in-process axum fixture and asserts the capture contract: a
//! non-pass ui check records the legacy failure captures plus one
//! post-step screenshot per browser-capable step and a synchronized
//! session-evidence document under the default `on-failure` policy;
//! `always` extends capture to passing checks; `off` records nothing;
//! and a captured trace still replays to the same verdict (the hub's
//! ingest revalidation must not be perturbed by capture observations).
//!
//! `#[ignore]`'d for the same reason as `engine_smoke.rs`: the
//! browser needs `npx playwright install chromium`. Locally:
//! `cargo test -p duhem-runtime --test capture_smoke -- --ignored`.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use duhem_actions::RunBrowser;
use duhem_evidence::{
    Event, EventPayload, ObservationValue, SESSION_EVIDENCE_OBSERVATION,
    STEP_SCREENSHOT_OBSERVATION, SessionEvidence, SqliteStore, Store, Trace, replay,
};
use duhem_judge::VerdictState;
use duhem_runtime::{CapturePolicy, Engine, RunOutcome};
use duhem_schema::VerificationDefinition;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const STATIC_HTML: &str = r#"<!doctype html>
<html><head><title>capture fixture</title></head>
<body><main><button id="create">Create</button></main></body></html>"#;

/// A page that, on load, POSTs to a failing endpoint with an auth
/// header and a credential-bearing body — the shape network capture
/// must record (the failing request) while redacting the secrets. The
/// secret literals are assembled from parts so they never appear
/// verbatim in this document's own response body (which capture
/// records as evidence); that isolates the test to request-side
/// redaction.
const NETWORK_HTML: &str = r#"<!doctype html>
<html><head><title>network fixture</title></head>
<body><main><button id="create">Create</button></main>
<script>
var token = 'sk' + '-' + 'secret';
var pw = 'hunter' + '2';
fetch('/api/data', {
  method: 'POST',
  headers: { 'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json' },
  body: JSON.stringify({ password: pw }),
}).catch(function () {});
</script>
</body></html>"#;

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
              timeout: 1s
            outputs:
              satisfied: satisfied
        assertions:
          - $steps.sso.outputs.satisfied == true
"#;

/// Navigate + assert a missing element; the `timeout` window gives the
/// on-load fetch time to round-trip and land in the recorder.
const NETWORK_YAML: &str = r#"
verification: capture smoke — network

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
              timeout: 3s
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
              timeout: 5s
            outputs:
              satisfied: satisfied
        assertions:
          - $steps.create.outputs.satisfied == true
"#;

const TIMED_OUT_YAML: &str = r#"
verification: capture smoke — timed-out ui step

inputs:
  fixture_url:
    type: string

criteria:
  - id: AC-1
    description: A timed-out browser action still leaves investigation evidence.
    checks:
      - id: AC-1.1
        steps:
          - id: open
            uses: ui/navigate
            with:
              url: $inputs.fixture_url
          - id: missing-click
            uses: ui/click
            with:
              role: button
              name: Never rendered
              timeout: 100ms
        assertions:
          - "false"
"#;

const INCONCLUSIVE_YAML: &str = r#"
verification: capture smoke — inconclusive browser check

inputs:
  fixture_url:
    type: string

criteria:
  - id: AC-1
    description: Any non-passing state retains the storyboard.
    checks:
      - id: AC-1.1
        steps:
          - id: open
            uses: ui/navigate
            with:
              url: $inputs.fixture_url
          - id: unreachable
            uses: api/call
            with:
              method: GET
              url: http://127.0.0.1:1/
              timeout: 100ms
            outputs:
              status: status
        assertions:
          - $steps.unreachable.outputs.status == 200
"#;

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

fn default_app() -> Router {
    Router::new().route("/", get(|| async { axum::response::Html(STATIC_HTML) }))
}

async fn start_app(app: Router) -> Fixture {
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

/// Run `yaml` under `policy` against the default static-page fixture.
async fn run_under(
    policy: CapturePolicy,
    yaml: &str,
) -> (RunOutcome, Vec<Event>, Arc<SqliteStore>, tempfile::TempDir) {
    run_app(default_app(), policy, yaml, false).await
}

/// Like [`run_under`] but with screencast recording on (#215).
async fn run_under_video(
    policy: CapturePolicy,
    yaml: &str,
) -> (RunOutcome, Vec<Event>, Arc<SqliteStore>, tempfile::TempDir) {
    run_app(default_app(), policy, yaml, true).await
}

/// Run `yaml` under `policy` against `app`, and hand back the outcome,
/// the full event stream, and the store (for blob fetches).
/// `record_video` toggles per-context screencast recording (#215).
async fn run_app(
    app: Router,
    policy: CapturePolicy,
    yaml: &str,
    record_video: bool,
) -> (RunOutcome, Vec<Event>, Arc<SqliteStore>, tempfile::TempDir) {
    let fx = start_app(app).await;
    let url = format!("http://{}/", fx.addr);
    let def = VerificationDefinition::from_yaml_str(yaml).expect("parse def");

    let browser = RunBrowser::launch(false)
        .await
        .expect("launch chromium (run `npx playwright install chromium`)")
        .with_video(record_video);

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
                value: ObservationValue::Blob { blob_sha256, .. },
                ..
            } if output_name.starts_with("capture/") => {
                Some((output_name.clone(), blob_sha256.clone()))
            }
            _ => None,
        })
        .collect()
}

async fn session_evidence(
    events: &[Event],
    store: &SqliteStore,
) -> (SessionEvidence, Vec<(u32, String)>) {
    let mut screenshots = Vec::new();
    let mut session_sha = None;
    for event in events {
        if let EventPayload::StepObservation {
            step_index,
            output_name,
            value: ObservationValue::Blob { blob_sha256, .. },
        } = &event.payload
        {
            if output_name == STEP_SCREENSHOT_OBSERVATION {
                screenshots.push((*step_index, blob_sha256.clone()));
            } else if output_name == SESSION_EVIDENCE_OBSERVATION {
                session_sha = Some(blob_sha256);
            }
        }
    }
    let bytes = store
        .get_blob(session_sha.expect("session evidence observation"))
        .await
        .expect("get session evidence")
        .expect("session evidence blob present");
    (
        serde_json::from_slice(&bytes).expect("valid session evidence"),
        screenshots,
    )
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn failing_ui_check_captures_screenshot_dom_and_network_by_default() {
    let (outcome, events, store, _tmp) = run_under(CapturePolicy::default(), FAILING_YAML).await;
    assert_eq!(outcome.verdict.state, VerdictState::Fail);

    let blobs = capture_blobs(&events);
    let names: Vec<&str> = blobs.iter().map(|(n, _)| n.as_str()).collect();
    // The page navigated, so its document response is in the buffer —
    // network + target-rect capture ride along with screenshot + DOM.
    assert_eq!(
        names,
        vec![
            "capture/screenshot",
            "capture/dom",
            "capture/network",
            "capture/target-rect",
            "capture/step-screenshot",
            "capture/step-screenshot",
            "capture/session-evidence"
        ],
        "expected legacy captures plus the complete storyboard, got {names:?}"
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

    // The target-rect records where the assertion looked. The "Sign in
    // with SSO" button is absent, so it's recorded found:false (never a
    // guessed box) — the dashboard renders a "target not found" note.
    let tr = store
        .get_blob(&blobs[3].1)
        .await
        .expect("get target-rect")
        .expect("target-rect blob present");
    let tr: serde_json::Value = serde_json::from_slice(&tr).expect("target-rect is JSON");
    assert_eq!(tr[0]["found"], false);
    assert!(
        tr[0]["selector"]
            .as_str()
            .unwrap()
            .contains("Sign in with SSO"),
        "target-rect carries the locator: {tr:?}"
    );

    // Both the successful navigation and the failed assertion retain a
    // post-step browser frame. The document references those exact
    // content-addressed blobs and expresses every boundary on one clock.
    let (session, screenshots) = session_evidence(&events, store.as_ref()).await;
    assert_eq!(session.version, 1);
    assert_eq!(session.clock, "monotonic");
    assert_eq!(
        screenshots.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(
        screenshots[0].1, screenshots[1].1,
        "unchanged frames are content-addressed to one blob"
    );
    assert_eq!(session.steps.len(), 2);
    for (step, (index, sha)) in session.steps.iter().zip(&screenshots) {
        assert_eq!(step.step_index, *index);
        assert_eq!(step.screenshot_sha256.as_deref(), Some(sha.as_str()));
        assert!(step.started_ms <= step.finished_ms);
        assert!(
            step.screenshot_ms
                .is_some_and(|ms| ms <= session.duration_ms),
            "step screenshot uses the session clock: {step:?}"
        );
    }

    // The reporter-facing failure carries the same refs.
    assert_eq!(outcome.failures.len(), 1);
    let caps = &outcome.failures[0].captures;
    assert_eq!(caps.len(), 5, "CheckFailure.captures = {caps:?}");
    assert_eq!(caps[0].kind, "capture/screenshot");
    assert_eq!(caps[1].kind, "capture/dom");
    assert_eq!(caps[2].kind, "capture/network");
    assert_eq!(caps[3].kind, "capture/target-rect");
    assert_eq!(caps[4].kind, "capture/session-evidence");
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
async fn timed_out_ui_step_still_retains_its_post_step_frame() {
    let (outcome, events, store, _tmp) = run_under(CapturePolicy::OnFailure, TIMED_OUT_YAML).await;
    assert_eq!(outcome.verdict.state, VerdictState::Fail);

    let (session, screenshots) = session_evidence(&events, store.as_ref()).await;
    assert_eq!(
        screenshots
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(session.steps.len(), 2);
    assert!(
        events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::StepFinished {
                step_index: 1,
                outcome: duhem_evidence::StepOutcome::Timeout,
            }
        )),
        "fixture must exercise the timeout path"
    );
    assert!(session.steps[1].screenshot_sha256.is_some());
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
async fn on_failure_retains_an_inconclusive_check_storyboard() {
    let (outcome, events, store, _tmp) =
        run_under(CapturePolicy::OnFailure, INCONCLUSIVE_YAML).await;
    assert!(
        matches!(outcome.verdict.state, VerdictState::Inconclusive(_)),
        "expected inconclusive, got {:?}",
        outcome.verdict.state
    );
    let (session, screenshots) = session_evidence(&events, store.as_ref()).await;
    assert_eq!(session.steps.len(), 2);
    assert_eq!(screenshots.len(), 2);
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn always_policy_captures_on_pass() {
    let (outcome, events, _store, _tmp) = run_under(CapturePolicy::Always, PASSING_YAML).await;
    assert_eq!(outcome.verdict.state, VerdictState::Pass);
    let names: Vec<String> = capture_blobs(&events).into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        vec![
            "capture/screenshot",
            "capture/dom",
            "capture/network",
            "capture/target-rect",
            "capture/step-screenshot",
            "capture/step-screenshot",
            "capture/session-evidence"
        ]
    );
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn failing_ui_check_with_video_records_a_webm_capture() {
    // Opt-in video (#215): a failing ui check under `on-failure` with
    // recording on yields a `capture/video` WebM alongside the other
    // captures — the screencast of the navigation + the failing wait.
    let (outcome, events, store, _tmp) =
        run_under_video(CapturePolicy::default(), FAILING_YAML).await;
    assert_eq!(outcome.verdict.state, VerdictState::Fail);

    let blobs = capture_blobs(&events);
    let video = blobs
        .iter()
        .find(|(n, _)| n == "capture/video")
        .expect("capture/video present when recording is on");
    // The blob is a real WebM (EBML magic), not a placeholder — playable
    // in the dashboard's <video>.
    let bytes = store
        .get_blob(&video.1)
        .await
        .expect("get video")
        .expect("video blob present");
    assert!(
        bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]),
        "video blob is not WebM/EBML"
    );
    // The synchronized session document points at that same WebM on
    // the shared monotonic clock. This is the contract the dashboard's
    // Replay model reads to reveal its Screenshot/Video toggle.
    let (session, _) = session_evidence(&events, store.as_ref()).await;
    let session_video = session
        .video
        .as_ref()
        .expect("session evidence carries the recorded video");
    assert_eq!(session_video.blob_sha256, video.1);
    assert_eq!(session_video.started_ms, 0.0);
    assert_eq!(session_video.finished_ms, session.duration_ms);
    // The reporter-facing failure carries the video ref too.
    assert!(
        outcome.failures[0]
            .captures
            .iter()
            .any(|c| c.kind == "capture/video"),
        "CheckFailure.captures should include the video"
    );
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn video_off_by_default_even_on_failure() {
    // Recording is a separate opt-in; the default policy captures the
    // screenshot/DOM/network/target-rect but never a video.
    let (_outcome, events, _store, _tmp) = run_under(CapturePolicy::default(), FAILING_YAML).await;
    assert!(
        !capture_blobs(&events)
            .iter()
            .any(|(n, _)| n == "capture/video"),
        "video must not be captured without --capture-video"
    );
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

fn network_app() -> Router {
    Router::new()
        .route("/", get(|| async { axum::response::Html(NETWORK_HTML) }))
        .route(
            "/api/data",
            post(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    r#"{"error":"charge declined"}"#,
                )
                    .into_response()
            }),
        )
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn network_capture_records_the_failing_request_and_redacts_secrets() {
    let (outcome, events, store, _tmp) =
        run_app(network_app(), CapturePolicy::default(), NETWORK_YAML, false).await;
    assert_eq!(outcome.verdict.state, VerdictState::Fail);

    let blobs = capture_blobs(&events);
    let (_, net_sha) = blobs
        .iter()
        .find(|(n, _)| n == "capture/network")
        .expect("network capture present");
    let bytes = store
        .get_blob(net_sha)
        .await
        .expect("get network blob")
        .expect("network blob present");

    // Valid HAR 1.2, and it carries the failing request.
    let har: serde_json::Value = serde_json::from_slice(&bytes).expect("valid HAR json");
    assert_eq!(har["log"]["version"], "1.2");
    let entries = har["log"]["entries"].as_array().unwrap();
    let api = entries
        .iter()
        .find(|e| {
            e["request"]["url"]
                .as_str()
                .is_some_and(|u| u.contains("/api/data"))
        })
        .expect("the /api/data request is recorded");
    assert_eq!(api["response"]["status"], 500);
    assert!(
        api["startedDateTime"]
            .as_str()
            .is_some_and(|value| value != "1970-01-01T00:00:00.000Z")
    );
    assert!(api["time"].as_f64().is_some_and(|value| value >= 0.0));
    assert!(
        api["timings"]["wait"]
            .as_f64()
            .is_some_and(|value| value >= 0.0)
    );
    assert!(
        api["timings"]["receive"]
            .as_f64()
            .is_some_and(|value| value >= 0.0)
    );
    // The response body — the repair signal — is captured.
    assert!(
        api["response"]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("charge declined"),
        "response body should carry the server error"
    );

    // Secrets are redacted: the auth header, and the credential-bearing
    // request body (auth-flow heuristic).
    let auth = api["request"]["headers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| {
            h["name"]
                .as_str()
                .unwrap()
                .eq_ignore_ascii_case("authorization")
        })
        .expect("authorization header recorded");
    assert_eq!(auth["value"], "<redacted>");
    assert_eq!(api["request"]["postData"]["text"], "<redacted>");

    // The request-side secrets never survive into the blob: the
    // fixture keeps them out of any response body, so their presence
    // here could only come from an unredacted header/postData.
    let full = String::from_utf8(bytes).unwrap();
    assert!(!full.contains("hunter2"), "password leaked into the HAR");
    assert!(!full.contains("sk-secret"), "token leaked into the HAR");

    let (session, _) = session_evidence(&events, store.as_ref()).await;
    let api = session
        .network
        .iter()
        .find(|entry| entry.url.contains("/api/data"))
        .expect("request timing is retained in session evidence");
    assert!(api.started_ms >= 0.0);
    assert!(api.duration_ms >= api.wait_ms);
    assert!(api.duration_ms >= api.receive_ms);
    assert!(api.started_ms + api.duration_ms <= session.duration_ms + 1.0);
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
