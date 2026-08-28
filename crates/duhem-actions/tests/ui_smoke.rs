//! End-to-end smoke for the `ui/*` catalog against an in-process
//! `axum`-served fixture. Drives a real Chromium via the official
//! Playwright Node sidecar (`crate::browser`; #71).
//!
//! Ignored in CI by default — running these requires
//! `npx playwright install chromium` (multi-hundred-MB download).
//! `just test browser-actions` runs them locally.
//!
//! Cases (per spec Plan on #12 and #37):
//!
//! - `navigate_succeeds_against_fixture`
//! - `click_present_button_succeeds`
//! - `assert_element_visible_present_satisfies`
//! - `assert_element_not_exists_with_present_alert_returns_false`
//! - `assert_element_timeout_returns_satisfied_false_quickly` —
//!   covers the §11.1 "wait-with-timeout, not poll" structural
//!   choice: a missed `timeout:` is *not* `Outcome::Timeout`. It
//!   yields `Outcome::Ok` with `satisfied: false` (a conclusive
//!   "we waited and it never appeared" observation), and elapsed
//!   wall time stays inside a loose multiple of `timeout:`.
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
use base64::Engine as _;
use duhem_actions::{
    Action, ActionCtx, AssertElement, AssertState, AssertUrl, Click, Extract, Navigate, Outcome,
    RunBrowser, Select, Type,
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
      <label><input id="updates" type="checkbox">Receive updates</label>
      <label for="name">Name</label>
      <input id="name" name="name" type="text" aria-label="Name">

      <label for="role">Role</label>
      <select id="role" name="role" aria-label="Role">
        <option value="">--</option>
        <option value="admin">Admin</option>
        <option value="editor">Editor</option>
        <option value="viewer">Viewer</option>
      </select>

      <!-- No textbox role: only reachable via { label: Password } (#256). -->
      <label for="password">Password</label>
      <input id="password" name="password" type="password">

      <input id="search" name="search" type="text" placeholder="Search projects">

      <button id="save" data-testid="save-btn" type="button">Save</button>
      <a class="item" href="/first">First</a>
      <a class="item" href="/second">Second</a>

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

fn static_data_url() -> String {
    format!(
        "data:text/html;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(STATIC_HTML)
    )
}

async fn fresh_browser() -> Arc<RunBrowser> {
    Arc::new(
        RunBrowser::launch(false)
            .await
            .expect("launch chromium (run `npx playwright install chromium`)"),
    )
}

fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    (
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
    )
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn headless_default_and_declared_viewports_size_screenshots() {
    for (viewport, expected) in [
        (duhem_schema::Viewport::default(), (1280, 720)),
        (
            duhem_schema::Viewport {
                width: 960,
                height: 540,
            },
            (960, 540),
        ),
    ] {
        let run = RunBrowser::launch_with_viewport(false, None, viewport)
            .await
            .unwrap();
        let check = run.open_check().await.unwrap();
        let shot = check.page.screenshot(5_000.0).await.unwrap();
        assert_eq!(png_dimensions(&shot), expected);
    }
}

#[tokio::test]
#[ignore = "requires a headed display and installed Chromium"]
async fn headed_viewport_follows_window_resize() {
    // CI runs this file with `--ignored`, and a headed Chromium needs a
    // display CI does not have. Skip loudly rather than fail — and
    // loudly rather than silently, so a green here is never mistaken
    // for evidence that window-tracking was exercised.
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!(
            "SKIPPED headed_viewport_follows_window_resize: no DISPLAY/WAYLAND_DISPLAY. \
             Headed window-tracking is NOT verified by this run."
        );
        return;
    }
    let run = RunBrowser::launch_with_viewport(true, None, duhem_schema::Viewport::default())
        .await
        .unwrap();
    let check = run.open_check().await.unwrap();
    let before: (u32, u32) = check
        .page
        .eval("[window.innerWidth, window.innerHeight]")
        .await
        .unwrap();
    check
        .page
        .eval::<serde_json::Value>("window.resizeTo(900, 700); null")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let after: (u32, u32) = check
        .page
        .eval("[window.innerWidth, window.innerHeight]")
        .await
        .unwrap();
    assert_ne!(
        after, before,
        "the page viewport must reflow with the window"
    );
    assert!(
        after.0 <= 900 && after.1 <= 700,
        "reported viewport: {after:?}"
    );
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
timeout: 2s
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
timeout: 500ms
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
timeout: 200ms
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
    // Loose upper bound — verifies we honored `timeout: 200ms` rather
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

/// The locator strategy union (#240, #462): `label` / `testid` / `css` /
/// `xpath` / `placeholder` each resolve a real element in Chromium. `label` is the
/// one `role`-only addressing can't do — it reaches a `type=password` input
/// (no `textbox` role), the crawlab-pro #256 unblock.
#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn locator_strategies_label_testid_css_xpath_placeholder_resolve() {
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

    // label → getByLabel reaches the password input.
    let r = Type
        .invoke(
            &ctx,
            &yaml(
                r#"
locator: { label: Password }
text: "s3cret!"
"#,
            ),
        )
        .await
        .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    let pw: String = check
        .page
        .eval("document.getElementById('password').value")
        .await
        .unwrap();
    assert_eq!(pw, "s3cret!");

    // testid → the `data-testid` attribute.
    let r = AssertElement
        .invoke(
            &ctx,
            &yaml(
                r#"
locator: { testid: save-btn }
expected: visible
timeout: 3s
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

    // css → raw CSS escape hatch.
    let r = AssertElement
        .invoke(
            &ctx,
            &yaml(
                r#"
locator: { css: "button#save" }
expected: visible
timeout: 3s
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

    // xpath → traverse upward from the stable button to its form ancestor,
    // the relationship CSS cannot express.
    let r = AssertElement
        .invoke(
            &ctx,
            &yaml(
                r#"
locator: { xpath: "//button[@id='save']/ancestor::form" }
expected: visible
timeout: 3s
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

    // placeholder → getByPlaceholder.
    let r = Type
        .invoke(
            &ctx,
            &yaml(
                r#"
locator: { placeholder: "Search projects" }
text: "crawlab"
"#,
            ),
        )
        .await
        .unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    let search: String = check
        .page
        .eval("document.getElementById('search').value")
        .await
        .unwrap();
    assert_eq!(search, "crawlab");
}

#[tokio::test]
#[ignore = "requires `npx playwright install chromium`"]
async fn extract_distinguishes_attribute_property_counts_and_direct_assertion() {
    let run = fresh_browser().await;
    let check = run.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    Navigate
        .invoke(&ctx, &yaml(&format!("url: {}", static_data_url())))
        .await
        .unwrap();
    Click
        .invoke(
            &ctx,
            &yaml(r#"{ role: checkbox, name: "Receive updates" }"#),
        )
        .await
        .unwrap();

    let property = Extract
        .invoke(
            &ctx,
            &yaml("locator: { css: '#updates' }\nproperty: checked"),
        )
        .await
        .unwrap();
    assert_eq!(
        property.outputs.get("value"),
        Some(&serde_json::json!(true))
    );
    let attribute = Extract
        .invoke(
            &ctx,
            &yaml("locator: { css: '#updates' }\nattribute: checked"),
        )
        .await
        .unwrap();
    assert_eq!(
        attribute.outputs.get("value"),
        Some(&serde_json::Value::Null)
    );
    assert_eq!(
        attribute.outputs.get("found"),
        Some(&serde_json::json!(true))
    );

    let missing = Extract
        .invoke(&ctx, &yaml("locator: { css: '.missing' }\nfield: text"))
        .await
        .unwrap();
    assert_eq!(
        missing.outputs.get("found"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(missing.outputs.get("value"), Some(&serde_json::Value::Null));

    let all = Extract
        .invoke(
            &ctx,
            &yaml("locator: { css: '.item' }\nfield: href\nall: true"),
        )
        .await
        .unwrap();
    assert_eq!(
        all.outputs.get("values"),
        Some(&serde_json::json!(["/first", "/second"]))
    );
    assert_eq!(all.outputs.get("count"), Some(&serde_json::json!(2)));
    let empty_all = Extract
        .invoke(
            &ctx,
            &yaml("locator: { css: '.missing' }\nfield: text\nall: true"),
        )
        .await
        .unwrap();
    assert_eq!(
        empty_all.outputs.get("values"),
        Some(&serde_json::json!([]))
    );
    assert_eq!(empty_all.outputs.get("count"), Some(&serde_json::json!(0)));
    let ambiguous = Extract
        .invoke(&ctx, &yaml("locator: { css: '.item' }\nfield: text"))
        .await
        .unwrap_err();
    assert!(ambiguous.to_string().contains("matched 2 elements"));
    assert!(ambiguous.to_string().contains("css .item"));

    let direct = AssertElement.invoke(&ctx, &yaml("locator: { role: checkbox, name: 'Receive updates' }\nexpect: { field: checked, equals: true }")).await.unwrap();
    assert_eq!(
        direct.outputs.get("satisfied"),
        Some(&serde_json::json!(true))
    );
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
            &yaml(r#"{ equals: "http://does.not/match", timeout: 200ms }"#),
        )
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(r.outcome, Outcome::Timeout);
    assert_eq!(
        r.outputs.get("satisfied").and_then(|v| v.as_bool()),
        Some(false)
    );
    // Loose upper bound — verifies we honored `timeout: 200ms`.
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
        .invoke(&ctx, &yaml(r#"{ matches: "/thanks", timeout: 2s }"#))
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
        .invoke(&ctx, &yaml(r#"{ state: loaded, timeout: 2s }"#))
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
timeout: 200ms
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
timeout: 1s
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
timeout: 200ms
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
timeout: 200ms
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
timeout: 1s
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
timeout: 200ms
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
