use std::process::Command;

fn fixture(up: &str, down: &str) -> String {
    format!(
        "verification: fixture values\nfixtures:\n  thing:\n    up:\n{up}    down:\n{down}criteria:\n  - id: AC-1\n    description: fixture expressions\n    checks:\n      - id: AC-1.1\n        needs: [thing]\n        assertions: [\"true\"]\n"
    )
}

const ROWS: &str =
    "      - id: rows\n        uses: cli/invoke\n        with: { command: [echo, '[]'] }\n";
const CLEANUP: &str = "      - uses: cli/invoke\n        with: { command: [echo, done] }\n";

fn validate(yaml: &str, expected: Option<(usize, usize, &str)>) {
    let tmp = tempfile::tempdir().unwrap();
    let leaf = tmp.path().join("fixture.yml");
    std::fs::write(&leaf, yaml).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_duhem"))
        .arg("validate")
        .arg(&leaf)
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    if let Some((line, col, message)) = expected {
        assert!(!output.status.success(), "{yaml}");
        assert_eq!(
            stderr,
            format!(
                "{}:{line}:{col}: [schema v{}] validation error: {message}\n",
                leaf.display(),
                duhem_schema::SCHEMA_VERSION
            )
        );
    } else {
        assert!(output.status.success(), "{stderr}\n{yaml}");
    }
}

fn with_leaf_rows(yaml: &str) -> String {
    yaml.replace(
        "fixtures:\n",
        "setup:\n  - id: rows\n    uses: cli/invoke\nfixtures:\n",
    )
}

fn fixture_setup_scope_message(site: &str, raw: &str) -> String {
    format!(
        "{site} `{raw}` references fixture step `rows`, which is declared but out of scope for `$setup`: only leaf-level `setup:` outputs are available through `$setup` in fixture bodies. Fixture `up:` outputs use `$fixture.thing.<up_step_id>.outputs.<output>` and may only be read from that fixture's own `down:` block (see §10.3.5)"
    )
}

#[test]
fn fixture_up_condition_checks_contract_outputs_at_exact_location() {
    let yaml = with_leaf_rows(&fixture(
        "      - uses: cli/invoke\n        if: $setup.rows.outputs.stdoutt\n",
        CLEANUP,
    ));
    validate(
        &yaml,
        Some((
            9,
            13,
            "fixture `thing` up step condition `$setup.rows.outputs.stdoutt` references undeclared output `stdoutt` on step `rows`",
        )),
    );
    validate(&yaml.replace("stdoutt", "stdout"), None);
}

#[test]
fn fixture_down_condition_checks_contract_outputs_at_exact_location() {
    for (root, line) in [("$setup.rows", 13), ("$fixture.thing.rows", 10)] {
        let yaml = fixture(
            ROWS,
            &format!("      - uses: cli/invoke\n        if: {root}.outputs.stdoutt\n"),
        );
        let yaml = if root == "$setup.rows" {
            with_leaf_rows(&yaml)
        } else {
            yaml
        };
        validate(
            &yaml,
            Some((
                line,
                13,
                &format!(
                    "fixture `thing` down step condition `{root}.outputs.stdoutt` references undeclared output `stdoutt` on step `rows`"
                ),
            )),
        );
        validate(&yaml.replace("stdoutt", "stdout"), None);
    }
}

#[test]
fn fixture_for_each_sources_check_contract_outputs_at_exact_location() {
    for (phase, root, line) in [
        ("up", "$setup.rows", 11),
        ("down", "$setup.rows", 12),
        ("down", "$fixture.thing.rows", 9),
    ] {
        let step = format!(
            "      - for_each: {root}.outputs.stdoutt\n        max: 5\n        uses: cli/invoke\n"
        );
        let yaml = if phase == "up" {
            fixture(&format!("{ROWS}{step}"), CLEANUP)
        } else {
            fixture(ROWS, &step)
        };
        let yaml = if root == "$setup.rows" {
            with_leaf_rows(&yaml)
        } else {
            yaml
        };
        validate(
            &yaml,
            Some((
                line,
                19,
                &format!(
                    "fixture `thing` {phase} step for_each source `{root}.outputs.stdoutt` references undeclared output `stdoutt` on step `rows`"
                ),
            )),
        );
        validate(&yaml.replace("stdoutt", "stdout"), None);
    }
}

#[test]
fn fixture_own_up_outputs_are_rejected_under_setup_at_exact_location() {
    for (key, label, col) in [("if", "condition", 13), ("for_each", "for_each source", 19)] {
        let extra = if key == "for_each" {
            "        max: 5\n"
        } else {
            ""
        };
        let step =
            format!("      - uses: cli/invoke\n        {key}: $setup.rows.outputs.stdout\n{extra}");
        for (phase, yaml, line) in [
            ("up", fixture(&format!("{ROWS}{step}"), CLEANUP), 9),
            ("down", fixture(ROWS, &step), 10),
            ("down", fixture(CLEANUP, &format!("{ROWS}{step}")), 12),
        ] {
            validate(
                &yaml,
                Some((
                    line,
                    col,
                    &fixture_setup_scope_message(
                        &format!("fixture `thing` {phase} step {label}"),
                        "$setup.rows.outputs.stdout",
                    ),
                )),
            );
        }
    }
}

#[test]
fn fixture_setup_references_to_self_or_later_steps_are_out_of_scope() {
    for key in ["if", "for_each"] {
        let extra = if key == "for_each" {
            "        max: 5\n"
        } else {
            ""
        };
        let step =
            format!("      - uses: cli/invoke\n        {key}: $setup.rows.outputs.stdout\n{extra}");
        let label = if key == "if" {
            "condition"
        } else {
            "for_each source"
        };
        let yaml = fixture(&format!("{step}{ROWS}"), CLEANUP);
        let col = if key == "if" { 13 } else { 19 };
        validate(
            &yaml,
            Some((
                6,
                col,
                &fixture_setup_scope_message(
                    &format!("fixture `thing` up step {label}"),
                    "$setup.rows.outputs.stdout",
                ),
            )),
        );
        validate(
            &yaml.replace("$setup.rows", "$setup.missing"),
            Some((
                6,
                col,
                &format!(
                    "fixture `thing` up step {label} `$setup.missing.outputs.stdout` references undeclared or forward step `missing`"
                ),
            )),
        );
    }
    validate(
        &fixture(
            "      - id: rows\n        uses: cli/invoke\n        if: $setup.rows.outputs.stdout\n",
            CLEANUP,
        ),
        Some((
            7,
            13,
            &fixture_setup_scope_message(
                "fixture `thing` up step condition",
                "$setup.rows.outputs.stdout",
            ),
        )),
    );
}

#[test]
fn fixture_ids_do_not_shadow_leaf_setup_outputs() {
    let yaml = with_leaf_rows(&fixture(
        "      - id: rows\n        uses: api/call\n      - uses: cli/invoke\n        if: $setup.rows.outputs.stdout\n",
        "      - id: rows\n        uses: api/call\n      - uses: cli/invoke\n        if: $setup.rows.outputs.stdout\n",
    ));
    validate(&yaml, None);
}

#[test]
fn fixture_scope_diagnostics_keep_their_message_and_location() {
    let up = format!(
        "{ROWS}      - uses: cli/invoke\n        with:\n          command: $fixture.thing.rows.outputs.stdout\n"
    );
    validate(
        &fixture(&up, CLEANUP),
        Some((
            10,
            20,
            "fixture `thing` up step `step 1` with:: fixture references are only valid in that fixture's own `down:` block",
        )),
    );
    for reference in [
        "$fixture.other.rows.outputs.stdout",
        "$fixture.thing.rows.outputs.stdoutt",
    ] {
        let down =
            format!("      - uses: cli/invoke\n        with:\n          command: {reference}\n");
        validate(
            &fixture(ROWS, &down),
            Some((
                11,
                20,
                &format!(
                    "fixture `thing` down step `step 0` references invalid fixture output `{reference}`"
                ),
            )),
        );
    }
}

#[test]
fn fixture_values_see_leaf_setup_without_exposing_it_as_fixture_outputs() {
    let yaml = fixture(
        "      - uses: cli/invoke\n        if: $setup.rows.outputs.stdout\n",
        "      - uses: cli/invoke\n        if: $setup.rows.outputs.stdout\n",
    );
    let yaml = yaml.replace(
        "fixtures:\n",
        "setup:\n  - id: rows\n    uses: cli/invoke\nfixtures:\n",
    );
    validate(&yaml, None);
    let yaml = yaml.replacen(
        "    down:\n      - uses: cli/invoke\n        if: $setup.rows.outputs.stdout",
        "    down:\n      - uses: cli/invoke\n        if: $fixture.thing.rows.outputs.stdout",
        1,
    );
    validate(
        &yaml,
        Some((
            12,
            13,
            "fixture `thing` down step condition `$fixture.thing.rows.outputs.stdout` references undeclared or forward step `rows`",
        )),
    );
}

#[test]
fn fixture_value_scope_errors_are_distinct_from_undeclared_outputs() {
    for (key, label, col) in [("if", "condition", 13), ("for_each", "for_each source", 19)] {
        let extra = if key == "for_each" {
            "        max: 5\n"
        } else {
            ""
        };
        let up = format!(
            "{ROWS}      - uses: cli/invoke\n        {key}: $fixture.thing.rows.outputs.stdout\n{extra}"
        );
        validate(
            &fixture(&up, CLEANUP),
            Some((
                9,
                col,
                &format!(
                    "fixture `thing` up step {label}: fixture references are only valid in that fixture's own `down:` block"
                ),
            )),
        );
        let down = format!(
            "      - uses: cli/invoke\n        {key}: $fixture.other.rows.outputs.stdout\n{extra}"
        );
        validate(
            &fixture(ROWS, &down),
            Some((
                10,
                col,
                &format!(
                    "fixture `thing` down step {label}: fixture references are only valid in that fixture's own `down:` block"
                ),
            )),
        );
    }
}
