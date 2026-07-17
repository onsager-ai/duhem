//! Round-trip the worked-example fixture from issue #12 through #8's
//! parser, then check shape compatibility of each `Step.with` against
//! a local mirror of the corresponding action's `With` (the real
//! `With` structs are private to each action module, by design — the
//! invariant under test is that the on-the-wire shape matches what
//! the action accepts, not the identity of the type).
//!
//! Runs in CI (no browser, no axum). The actual Playwright smoke
//! lives in `ui_smoke.rs` and is `#[ignore]`'d by default.

use duhem_actions::{ExistenceState, Locator};
use duhem_schema::VerificationDefinition;

const FIXTURE: &str = include_str!("fixtures/static-page.yml");

// Note on `validate()`: the v0.1 validator (`duhem-schema::validate`)
// requires every `$steps.<id>.outputs.<name>` to resolve against an
// explicit `Step.outputs` extraction map. `ui/assert-element` produces
// `satisfied` and `count` *natively* (from the action, not from an
// extraction expression). Bridging native action outputs into the
// validator needs the action-type catalog to be enforced — a separate
// spec (`spec(schema): catalog-aware validation`). Until then this
// fixture exercises parse + per-action `With` shapes only.

#[test]
fn fixture_parses_at_schema_layer() {
    let def = VerificationDefinition::from_yaml_str(FIXTURE).expect("parse");
    assert_eq!(def.verification, "Static page smoke");
    // AC-1 shipped with #12; AC-2 was added in #37 to exercise the
    // rest-of-slice ui/* actions on the same fixture.
    assert_eq!(def.criteria.len(), 2);
    assert_eq!(def.criteria[0].id, "AC-1");
    assert_eq!(def.criteria[0].checks.len(), 1);
    assert_eq!(def.criteria[0].checks[0].steps.len(), 3);
    assert_eq!(
        def.criteria[0].checks[0].steps[2].id.as_deref(),
        Some("banner")
    );
    // Run-level setup block (issue #20).
    assert_eq!(def.setup.len(), 2);
    assert_eq!(def.setup[0].uses, "ui/navigate");
    assert_eq!(def.setup[1].id.as_deref(), Some("probe"));
    assert_eq!(def.setup[1].uses, "ui/assert-element");

    // AC-2: the four new actions land in the fixture.
    assert_eq!(def.criteria[1].id, "AC-2");
    let ac2_steps = &def.criteria[1].checks[0].steps;
    let uses: Vec<&str> = ac2_steps.iter().map(|s| s.uses.as_str()).collect();
    assert_eq!(
        uses,
        vec![
            "ui/navigate",
            "ui/type",
            "ui/select",
            "ui/click",
            "ui/assert-url",
            "ui/assert-state",
        ]
    );
}

#[test]
fn each_step_with_deserializes_into_action_with() {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct NavWith {
        url: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ClickWith {
        role: String,
        name: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AssertWith {
        locator: Locator,
        expected: ExistenceState,
        within: Option<String>,
    }

    let def = VerificationDefinition::from_yaml_str(FIXTURE).unwrap();
    let steps = &def.criteria[0].checks[0].steps;

    let nav: NavWith = serde_yml::from_value(steps[0].with.clone()).expect("navigate");
    assert_eq!(nav.url, "$inputs.fixture_url");

    let click: ClickWith = serde_yml::from_value(steps[1].with.clone()).expect("click");
    assert_eq!(click.role, "button");
    assert_eq!(click.name.as_deref(), Some("Create"));

    let assertion: AssertWith =
        serde_yml::from_value(steps[2].with.clone()).expect("assert-element");
    assert_eq!(assertion.expected, ExistenceState::Visible);
    assert_eq!(assertion.locator.role.as_deref(), Some("alert"));
    assert_eq!(assertion.locator.text.as_deref(), Some("Created"));
    assert_eq!(assertion.within.as_deref(), Some("2s"));
}
