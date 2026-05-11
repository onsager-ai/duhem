//! End-to-end smoke for `ui/navigate` + `ui/click` + `ui/assert-element`
//! against an in-process `axum`-served fixture. Drives a real
//! Chromium via the `playwright` crate.
//!
//! Ignored in CI by default — running these requires
//! `npx playwright install chromium` (multi-hundred-MB download).
//! The `just test-ui` recipe runs them locally.
//!
//! Cases (per spec Plan):
//!
//! - `navigate_succeeds_against_fixture`
//! - `click_present_button_succeeds`
//! - `assert_element_visible_present_satisfies`
//! - `assert_element_not_exists_with_present_alert_returns_false`
//! - `assert_element_timeout_returns_satisfied_false_quickly`
//!   covers the §11.1 "wait-with-timeout, not poll" structural
//!   choice: a missed `within:` is *not* `Outcome::Timeout`. It
//!   yields `Outcome::Ok` with `satisfied: false` (a conclusive
//!   "we waited and it never appeared" observation), and elapsed
//!   wall time stays inside a loose multiple of `within:`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::get;
use duhem_actions::{Action, ActionCtx, AssertElement, Click, Navigate, Outcome, RunBrowser};
use serde_yml::Value;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const STATIC_HTML: &str = r#"<!doctype html>
<html><head><title>fixture</title></head>
<body>
  <main>
    <button id="create" onclick="
      var b = document.createElement('div');
      b.setAttribute('role', 'alert');
      b.textContent = 'Created';
      document.body.appendChild(b);
    ">Create</button>
  </main>
</body></html>"#;

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

fn url(fx: &Fixture) -> String {
    format!("http://{}/", fx.addr)
}

async fn fresh_browser() -> Arc<RunBrowser> {
    Arc::new(
        RunBrowser::launch(false)
            .await
            .expect("launch chromium (run `npx playwright install chromium`)"),
    )
}

fn yaml(s: &str) -> Value {
    serde_yml::from_str(s).unwrap()
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn navigate_succeeds_against_fixture() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: &check.page,
        step_index: 0,
    };
    let r = Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn click_present_button_succeeds() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: &check.page,
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();
    let r = Click
        .invoke(&ctx, &yaml(r#"{ role: button, name: Create }"#))
        .await
        .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn assert_element_visible_present_satisfies() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: &check.page,
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();
    Click
        .invoke(&ctx, &yaml(r#"{ role: button, name: Create }"#))
        .await
        .unwrap();
    let r = AssertElement
        .invoke(
            &ctx,
            &yaml(
                r#"
locator: { role: alert, text: "Created" }
expected: visible
within: 2s
"#,
            ),
        )
        .await
        .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    assert_eq!(
        r.outputs.get("satisfied").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn assert_element_not_exists_with_present_alert_returns_false() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: &check.page,
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();
    Click
        .invoke(&ctx, &yaml(r#"{ role: button, name: Create }"#))
        .await
        .unwrap();
    let r = AssertElement
        .invoke(
            &ctx,
            &yaml(
                r#"
locator: { role: alert, text: "Created" }
expected: not_exists
within: 500ms
"#,
            ),
        )
        .await
        .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    assert_eq!(
        r.outputs.get("satisfied").and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn assert_element_timeout_returns_satisfied_false_quickly() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: &check.page,
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();
    let started = Instant::now();
    let r = AssertElement
        .invoke(
            &ctx,
            &yaml(
                r#"
locator: { role: alert, text: "never" }
expected: visible
within: 200ms
"#,
            ),
        )
        .await
        .unwrap();
    let elapsed = started.elapsed();
    // Wait-with-timeout, not hard-fail: Outcome stays Ok and the
    // observation is conclusive (`satisfied: false`).
    assert_eq!(r.outcome, Outcome::Ok);
    assert_eq!(
        r.outputs.get("satisfied").and_then(|v| v.as_bool()),
        Some(false)
    );
    // Loose upper bound — verifies we honored `within: 200ms` rather
    // than the 5s default.
    assert!(
        elapsed < Duration::from_millis(2_000),
        "elapsed = {elapsed:?}"
    );
}
