//! Real browser coverage for `ui/emulate` (#580 S3): read-back, layout
//! effect, and reset between sibling checks and retry attempts.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use axum::{Router, extract::RawQuery, http::StatusCode, response::Html, routing::get};
use duhem_actions::{Action, ActionCtx, ActionResult, Emulate, Outcome, RunBrowser};
use duhem_evidence::SqliteStore;
use duhem_judge::{InconclusiveCause, VerdictState};
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use serde_json::json;
use tokio::{net::TcpListener, task::JoinHandle};

const EXAMPLE: &str = include_str!("../../../verifications/emulate-example/duhem.yml");
const PAGE: &str = r#"<!doctype html><html><head><style>
body { background: #fff; color: #111; }
.dark-only, .narrow-only { display: none; }
@media (prefers-color-scheme: dark) {
  body { background: #111; color: #eee; }
  .dark-only { display: block; }
  .light-only { display: none; }
}
@media (max-width: 600px) {
  .narrow-only { display: block; }
  .wide-only { display: none; }
}
</style></head><body>
<h1>Layout probe</h1>
<p class="wide-only" data-testid="wide-layout">Wide layout</p>
<p class="narrow-only" data-testid="narrow-layout">Narrow layout</p>
<p class="light-only" data-testid="light-theme">Light theme</p>
<p class="dark-only" data-testid="dark-theme">Dark theme</p>
<script>new Image().src = '/log?w=' + innerWidth + '&dark=' +
  matchMedia('(prefers-color-scheme: dark)').matches + '&n=' + Math.random()</script>
</body></html>"#;

struct Fixture {
    url: String,
    log: Arc<Mutex<Vec<String>>>,
    server: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn fixture() -> Fixture {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    let app = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route(
            "/log",
            get(move |RawQuery(query): RawQuery| {
                let sink = sink.clone();
                async move {
                    // Drop the cache-busting `&n=` nonce; keep the probe facts.
                    let query = query.unwrap_or_default();
                    let facts = query.split("&n=").next().unwrap_or_default();
                    sink.lock().unwrap().push(facts.to_string());
                    StatusCode::NO_CONTENT
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fixture { url, log, server }
}
fn yaml(raw: &str) -> serde_yml::Value {
    serde_yml::from_str(raw).unwrap()
}
async fn emulate(ctx: &ActionCtx<'_>, with: &str) -> ActionResult {
    Emulate.invoke(ctx, &yaml(with)).await.unwrap()
}
async fn run(engine: &mut Engine, vd: &str, url: &str) -> duhem_runtime::RunOutcome {
    let def = VerificationDefinition::from_yaml_str(vd).unwrap();
    engine
        .run_with_metadata(
            &def,
            BTreeMap::from([("fixture_url".into(), serde_json::json!(url))]),
        )
        .await
        .unwrap()
}
async fn engine(browser: RunBrowser, tmp: &tempfile::TempDir) -> Engine {
    let store = Arc::new(SqliteStore::open(tmp.path().join("runs.db")).await.unwrap());
    Engine::new().with_browser(browser).with_store(store)
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn worked_vd_applies_emulation_then_sibling_check_sees_defaults() {
    let fx = fixture().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut engine = engine(RunBrowser::launch(false).await.unwrap(), &tmp).await;
    let outcome = run(&mut engine, EXAMPLE, &fx.url).await;
    assert_eq!(
        outcome.verdict.state,
        VerdictState::Pass,
        "{:?}",
        outcome.verdict
    );
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn reads_back_effective_settings_on_blank_page_and_next_context_is_default() {
    let browser = RunBrowser::launch(false).await.unwrap();
    let first = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&first.page),
        step_index: 0,
    };
    // No navigation yet: the page is still about:blank.
    let dark = emulate(
        &ctx,
        "viewport: { width: 390, height: 844 }\ncolor_scheme: dark",
    )
    .await;
    assert!(matches!(dark.outcome, Outcome::Ok));
    assert_eq!(
        dark.outputs["viewport"],
        json!({"width": 390, "height": 844})
    );
    assert_eq!(dark.outputs["color_scheme"], json!("dark"));
    // Each field is independently optional; the other keeps its value.
    let light = emulate(&ctx, "color_scheme: light").await;
    assert_eq!(
        light.outputs["viewport"],
        json!({"width": 390, "height": 844})
    );
    assert_eq!(light.outputs["color_scheme"], json!("light"));
    let resized = emulate(&ctx, "viewport: { width: 800, height: 600 }").await;
    assert_eq!(
        resized.outputs["viewport"],
        json!({"width": 800, "height": 600})
    );
    assert_eq!(resized.outputs["color_scheme"], json!("light"));
    // Dark again, so the next context has something to not inherit.
    emulate(&ctx, "color_scheme: dark").await;
    first.close(false, 0).await.unwrap();

    let second = browser.open_check().await.unwrap();
    let probe: serde_json::Value = second
        .page
        .eval(
            "({ w: window.innerWidth, h: window.innerHeight, \
             dark: matchMedia('(prefers-color-scheme: dark)').matches })",
        )
        .await
        .unwrap();
    assert_eq!(probe, json!({"w": 1280, "h": 720, "dark": false}));
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn invalid_emulation_fails_usefully() {
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    for bad in [
        "{}",
        "viewport: { width: 0, height: 10 }",
        "color_scheme: sepia",
    ] {
        let result = Emulate.invoke(&ctx, &yaml(bad)).await;
        assert!(result.is_err(), "{bad}");
    }
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn every_retry_attempt_starts_from_default_viewport_and_color_scheme() {
    let fx = fixture().await;
    let tmp = tempfile::tempdir().unwrap();
    let defaults = serde_yml::from_str("timeout: 5s\nretry: { max: 1, backoff: linear }").unwrap();
    let mut engine = engine(RunBrowser::launch(false).await.unwrap(), &tmp)
        .await
        .with_defaults(&defaults);
    // The check emulates, then times out (retryable). The page logs the
    // viewport and scheme it saw at load, which is before each attempt's
    // own emulation.
    let vd = r#"
verification: Retry attempts do not inherit emulation
inputs:
  fixture_url: { type: string }
criteria:
  - id: AC-1
    description: Emulation is discarded between attempts.
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/navigate
            with: { url: $inputs.fixture_url }
          - uses: ui/emulate
            with: { viewport: { width: 390, height: 844 }, color_scheme: dark }
          - uses: cli/invoke
            with: { command: [sleep, "1"], timeout: 1ms }
        assertions: ["true"]
"#;
    let outcome = run(&mut engine, vd, &fx.url).await;
    assert!(
        matches!(
            outcome.verdict.state,
            VerdictState::Inconclusive(InconclusiveCause::Timeout)
        ),
        "{:?}",
        outcome.verdict
    );
    let log = fx.log.lock().unwrap().clone();
    assert_eq!(
        log,
        vec!["w=1280&dark=false"; 2],
        "{log:?}; verdict: {:?}",
        outcome.verdict
    );
}
