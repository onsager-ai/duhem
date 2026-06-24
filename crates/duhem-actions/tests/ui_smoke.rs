//! End-to-end smoke for the `ui/*` catalog against an in-process
//! `axum`-served fixture. Drives a real Chromium via the official
//! Playwright Node sidecar (`crate::browser`; #71).
//!
//! Ignored in CI by default — running these requires
//! `npx playwright install chromium` (multi-hundred-MB download).
//! The `just test-ui` recipe runs them locally.
//!
//! Cases (per spec Plan on #12 and #37):
//!
//! - `navigate_succeeds_against_fixture`
//! - `click_present_button_succeeds`
//! - `assert_element_visible_present_satisfies`
//! - `assert_element_not_exists_with_present_alert_returns_false`
//! - `assert_element_timeout_returns_satisfied_false_quickly` —
//!   covers the §11.1 "wait-with-timeout, not poll" structural
//!   choice: a missed `within:` is *not* `Outcome::Timeout`. It
//!   yields `Outcome::Ok` with `satisfied: false` (a conclusive
//!   "we waited and it never appeared" observation), and elapsed
//!   wall time stays inside a loose multiple of `within:`.
//! - `type_fills_input_then_assert_element_reads_it_back`
//! - `select_by_value_label_index_dispatches_to_playwright`
//! - `assert_url_passes_on_navigation_and_times_out_on_stale_url`
//! - `assert_state_loaded_resolves_when_ready_state_is_complete`
//! - `assert_state_authenticated_observes_cookie_marker_present_and_absent`
//! - `assert_state_signed_out_observes_local_storage_marker_present_and_absent`

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::get;
use duhem_actions::{
    Action, ActionCtx, AssertElement, AssertState, AssertUrl, Click, Navigate, Outcome, RunBrowser,
    Select, Type,
};
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

    <form action="/thanks" method="get">
      <label for="name">Name</label>
      <input id="name" name="name" type="text" aria-label="Name">

      <label for="role">Role</label>
      <select id="role" name="role" aria-label="Role">
        <option value="">--</option>
        <option value="admin">Admin</option>
        <option value="editor">Editor</option>
        <option value="viewer">Viewer</option>
      </select>

      <button type="submit">Submit</button>
    </form>
  </main>
</body></html>"#;

const THANKS_HTML: &str = r#"<!doctype html>
<html><head><title>thanks</title></head>
<body><h1>Thanks</h1></body></html>"#;

struct Fixture {
    addr: SocketAddr,
    _server: JoinHandle<()>,
}

async fn start_fixture() -> Fixture {
    let app = Router::new()
        .route("/", get(|| async { axum::response::Html(STATIC_HTML) }))
        .route(
            "/thanks",
            get(|| async { axum::response::Html(THANKS_HTML) }),
        );
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
        page: Some(&check.page),
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
        page: Some(&check.page),
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
        page: Some(&check.page),
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
        page: Some(&check.page),
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
        page: Some(&check.page),
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

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn type_fills_input_then_assert_element_reads_it_back() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();
    let r = Type
        .invoke(
            &ctx,
            &yaml(
                r#"
locator: { role: textbox, name: Name }
text: "Alice"
"#,
            ),
        )
        .await
        .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);

    // Read back via the DOM — the input now holds "Alice".
    let value: String = check
        .page
        .eval("document.getElementById('name').value")
        .await
        .unwrap();
    assert_eq!(value, "Alice");
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn select_by_value_label_index_dispatches_to_playwright() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;

    for (by_yaml, expected) in [
        (r#"by: { value: editor }"#, "editor"),
        (r#"by: { label: "Admin" }"#, "admin"),
        // Index 3 in the option list: [--, admin, editor, viewer].
        (r#"by: { index: 3 }"#, "viewer"),
    ] {
        let check = run.open_check().await.unwrap();
        let ctx = ActionCtx {
            page: Some(&check.page),
            step_index: 0,
        };
        Navigate
            .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
            .await
            .unwrap();
        let with = format!("locator: {{ role: combobox, name: Role }}\n{}\n", by_yaml);
        let r = Select.invoke(&ctx, &yaml(&with)).await.unwrap();
        assert_eq!(r.outcome, Outcome::Ok, "by_yaml = {by_yaml}");

        let observed: String = check
            .page
            .eval("document.getElementById('role').value")
            .await
            .unwrap();
        assert_eq!(observed, expected, "by_yaml = {by_yaml}");
    }
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn assert_url_passes_on_navigation_and_times_out_on_stale_url() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();

    // Equals against the freshly loaded URL — should pass immediately.
    let landing = url(&fx);
    let r = AssertUrl
        .invoke(&ctx, &yaml(&format!(r#"{{ equals: "{landing}" }}"#)))
        .await
        .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    assert_eq!(
        r.outputs.get("satisfied").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        r.outputs.get("actual").and_then(|v| v.as_str()),
        Some(landing.as_str())
    );

    // Stale-URL timeout: the URL will never match, so the action
    // must time out within ~200ms wall clock.
    let started = Instant::now();
    let r = AssertUrl
        .invoke(
            &ctx,
            &yaml(r#"{ equals: "http://does.not/match", within: 200ms }"#),
        )
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(r.outcome, Outcome::Timeout);
    assert_eq!(
        r.outputs.get("satisfied").and_then(|v| v.as_bool()),
        Some(false)
    );
    // Loose upper bound — verifies we honored `within: 200ms`.
    assert!(
        elapsed < Duration::from_millis(2_000),
        "elapsed = {elapsed:?}"
    );

    // Click-then-assert-url against a regex matcher — exercises the
    // `matches:` shape on a real navigation.
    Click
        .invoke(&ctx, &yaml(r#"{ role: button, name: Submit }"#))
        .await
        .unwrap();
    let r = AssertUrl
        .invoke(&ctx, &yaml(r#"{ matches: "/thanks", within: 2s }"#))
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
async fn assert_state_loaded_resolves_when_ready_state_is_complete() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();
    let r = AssertState
        .invoke(&ctx, &yaml(r#"{ state: loaded, within: 2s }"#))
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
async fn assert_state_authenticated_observes_cookie_marker_present_and_absent() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();

    // Before the cookie is set: authenticated → false.
    let r = AssertState
        .invoke(
            &ctx,
            &yaml(
                r#"
state: authenticated
marker: { kind: cookie, name: "session" }
within: 200ms
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

    // Add the cookie via JS on the current origin (the sidecar Page
    // exposes no direct cookie injection; `document.cookie` suffices
    // for a non-HttpOnly marker, and `context.cookies()` then sees it).
    let _: serde_json::Value = check
        .page
        .eval("document.cookie = 'session=deadbeef'")
        .await
        .unwrap();

    let r = AssertState
        .invoke(
            &ctx,
            &yaml(
                r#"
state: authenticated
marker: { kind: cookie, name: "session" }
within: 1s
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

    // signed_out is the inverse — with the cookie present, false.
    let r = AssertState
        .invoke(
            &ctx,
            &yaml(
                r#"
state: signed_out
marker: { kind: cookie, name: "session" }
within: 200ms
"#,
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        r.outputs.get("satisfied").and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn assert_state_signed_out_observes_local_storage_marker_present_and_absent() {
    let fx = start_fixture().await;
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", url(&fx))))
        .await
        .unwrap();

    // Empty local storage → signed_out true.
    let r = AssertState
        .invoke(
            &ctx,
            &yaml(
                r#"
state: signed_out
marker: { kind: local_storage, name: "auth_token" }
within: 200ms
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

    // Set the key — signed_out flips to false, authenticated true.
    let _: serde_json::Value = check
        .page
        .eval("localStorage.setItem('auth_token', 'x')")
        .await
        .unwrap();

    let r = AssertState
        .invoke(
            &ctx,
            &yaml(
                r#"
state: authenticated
marker: { kind: local_storage, name: "auth_token" }
within: 1s
"#,
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        r.outputs.get("satisfied").and_then(|v| v.as_bool()),
        Some(true)
    );

    let r = AssertState
        .invoke(
            &ctx,
            &yaml(
                r#"
state: signed_out
marker: { kind: local_storage, name: "auth_token" }
within: 200ms
"#,
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        r.outputs.get("satisfied").and_then(|v| v.as_bool()),
        Some(false)
    );
}
