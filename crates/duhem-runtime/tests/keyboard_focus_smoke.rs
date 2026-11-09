//! Real browser coverage for #580 S1: `ui/press`, `ui/focus` and
//! `expected: focused | not_focused` against a local fixture.
use std::collections::BTreeMap;

use axum::{Router, response::Html, routing::get};
use duhem_actions::{Action, ActionCtx, AssertElement, Focus, Navigate, Press, RunBrowser};
use duhem_judge::VerdictState;
use duhem_runtime::Engine;
use duhem_schema::VerificationDefinition;
use serde_yml::Value;
use tokio::{net::TcpListener, task::JoinHandle};

const EXAMPLE: &str = include_str!("../../../verifications/keyboard-focus-example/duhem.yml");
const PAGE: &str = r#"<!doctype html><html><body>
<h1>Keyboard fixture</h1>
<button onclick="document.querySelector('[role=menu]').hidden=false">Options</button>
<input aria-label="Search">
<div role="menu" hidden><button role="menuitem">Copy</button><button role="menuitem">Paste</button></div>
<iframe data-testid="inner" src="/inner" style="width:300px;height:80px"></iframe>
<script>
document.addEventListener('keydown', e => {
  const menu = document.querySelector('[role=menu]');
  if (e.key === 'ArrowDown' && !menu.hidden) {
    e.preventDefault();
    document.querySelector('[role=menuitem]').focus();
  }
});
</script>
</body></html>"#;
const INNER: &str = r#"<!doctype html><html><body><input aria-label="Inner field"></body></html>"#;

struct Fixture {
    url: String,
    server: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn fixture() -> Fixture {
    let app = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/inner", get(|| async { Html(INNER) }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fixture { url, server }
}
fn yaml(raw: &str) -> Value {
    serde_yml::from_str(raw).unwrap()
}
async fn navigate(ctx: &ActionCtx<'_>, url: &str) {
    Navigate
        .invoke(
            ctx,
            &serde_yml::to_value(serde_json::json!({"url":url})).unwrap(),
        )
        .await
        .unwrap();
}
async fn satisfied(ctx: &ActionCtx<'_>, locator: &str, expected: &str) -> bool {
    let with = yaml(&format!(
        "locator: {locator}\nexpected: {expected}\ntimeout: 400ms"
    ));
    let result = AssertElement.invoke(ctx, &with).await.unwrap();
    result.outputs["satisfied"].as_bool().unwrap()
}

const OPTIONS: &str = "{role: button, name: Options}";
const SEARCH: &str = "{role: textbox, name: Search}";
const INNER_FIELD: &str = "{frame: {testid: inner}, role: textbox, name: Inner field}";

#[tokio::test]
#[ignore = "requires Chromium"]
async fn worked_vd_executes_keyboard_activation_and_focus() {
    let fx = fixture().await;
    let def = VerificationDefinition::from_yaml_str(EXAMPLE).unwrap();
    let browser = RunBrowser::launch(false).await.unwrap();
    let mut engine = Engine::new().with_browser(browser);
    let outcome = engine
        .run_with_metadata(
            &def,
            BTreeMap::from([("fixture_url".into(), serde_json::json!(fx.url))]),
        )
        .await
        .unwrap();
    assert_eq!(
        outcome.verdict.state,
        VerdictState::Pass,
        "{:?}",
        outcome.verdict
    );
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn tab_moves_focus_and_focused_distinguishes_visible_controls() {
    let fx = fixture().await;
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    navigate(&ctx, &fx.url).await;
    Focus
        .invoke(&ctx, &yaml(&format!("locator: {OPTIONS}")))
        .await
        .unwrap();
    assert!(satisfied(&ctx, OPTIONS, "focused").await);
    assert!(!satisfied(&ctx, OPTIONS, "not_focused").await);
    // Negative control: visible, present, but not the active element.
    assert!(satisfied(&ctx, SEARCH, "visible").await);
    assert!(!satisfied(&ctx, SEARCH, "focused").await);
    assert!(satisfied(&ctx, SEARCH, "not_focused").await);

    let pressed = Press.invoke(&ctx, &yaml("key: Tab")).await.unwrap();
    assert_eq!(pressed.outputs["target"]["tag"], "button");
    assert_eq!(pressed.outputs["target"]["role"], "button");
    assert_eq!(pressed.outputs["target"]["name"], "Options");
    assert!(satisfied(&ctx, SEARCH, "focused").await);
    assert!(!satisfied(&ctx, OPTIONS, "focused").await);
    assert!(satisfied(&ctx, OPTIONS, "visible").await);
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn enter_activates_focused_button_and_locator_press_describes_element() {
    let fx = fixture().await;
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    navigate(&ctx, &fx.url).await;
    assert!(!satisfied(&ctx, "{role: menu}", "visible").await);
    let pressed = Press
        .invoke(&ctx, &yaml(&format!("key: Enter\nlocator: {OPTIONS}")))
        .await
        .unwrap();
    assert_eq!(pressed.outputs["target"]["name"], "Options");
    assert!(satisfied(&ctx, "{role: menu}", "visible").await);
    Press.invoke(&ctx, &yaml("key: ArrowDown")).await.unwrap();
    assert!(satisfied(&ctx, "{role: menuitem, name: Copy}", "focused").await);
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn focus_in_frame_is_distinct_from_parent_and_press_target_is_deepest() {
    let fx = fixture().await;
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    navigate(&ctx, &fx.url).await;
    Focus
        .invoke(&ctx, &yaml(&format!("locator: {INNER_FIELD}")))
        .await
        .unwrap();
    assert!(satisfied(&ctx, INNER_FIELD, "focused").await);
    assert!(satisfied(&ctx, OPTIONS, "not_focused").await);
    assert!(satisfied(&ctx, SEARCH, "not_focused").await);
    let pressed = Press.invoke(&ctx, &yaml("key: Shift")).await.unwrap();
    assert_eq!(pressed.outputs["target"]["tag"], "input");
    assert_eq!(pressed.outputs["target"]["name"], serde_json::Value::Null);
    assert_eq!(pressed.outputs["target"]["role"], "textbox");

    // Moving focus to the parent leaves the frame's activeElement set,
    // but its document no longer has focus.
    Focus
        .invoke(&ctx, &yaml(&format!("locator: {OPTIONS}")))
        .await
        .unwrap();
    assert!(satisfied(&ctx, OPTIONS, "focused").await);
    assert!(!satisfied(&ctx, INNER_FIELD, "focused").await);
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn press_target_is_body_when_nothing_is_focused() {
    let fx = fixture().await;
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    navigate(&ctx, &fx.url).await;
    let pressed = Press.invoke(&ctx, &yaml("key: Shift")).await.unwrap();
    let target = &pressed.outputs["target"];
    assert_eq!(target["tag"], "body");
    assert!(target["id"].is_null() && target["role"].is_null());
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn invalid_key_and_ambiguous_or_missing_targets_fail_usefully() {
    let fx = fixture().await;
    let browser = RunBrowser::launch(false).await.unwrap();
    let check = browser.open_check().await.unwrap();
    let ctx = ActionCtx {
        page: Some(&check.page),
        step_index: 0,
    };
    navigate(&ctx, &fx.url).await;
    let error = Press
        .invoke(&ctx, &yaml("key: NotARealKey"))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("ui/press") && error.contains("NotARealKey"),
        "{error}"
    );
    let error = Press
        .invoke(&ctx, &yaml("key: ''"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("key"), "{error}");

    let missing = Focus
        .invoke(
            &ctx,
            &yaml("locator: {role: button, name: Missing}\ntimeout: 250ms"),
        )
        .await
        .unwrap();
    assert!(matches!(missing.outcome, duhem_actions::Outcome::Timeout));
    // Absent elements are never "not focused".
    assert!(!satisfied(&ctx, "{role: button, name: Missing}", "not_focused").await);

    let error = AssertElement
        .invoke(
            &ctx,
            &yaml("locator: {role: button}\nexpected: focused\ntimeout: 250ms"),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("expected exactly one"), "{error}");
}
