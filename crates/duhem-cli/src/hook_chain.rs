//! Resolved lifecycle hook chain for one check (#441 Part B).
//!
//! `setup:`/`teardown:` can now be declared at four places — leaf,
//! criterion, check, and named fixtures (`needs:`) — plus `provision:`
//! outside a check's own chain entirely. #441 is explicit that this
//! lookup surface is the feature's real cost: "when something runs
//! unexpectedly, that is a lot of files to read." This module answers
//! that mechanically, by walking the static Verification Definition
//! for one check and naming, in execution order, every hook that
//! applies to it and where it was declared.
//!
//! The resolution is static (from the authored document), not a
//! replay of what actually happened on a given run — which is the
//! right question for the target scenario: "I did not expect this
//! check to have preconditions/cleanup at all, where do they come
//! from?" A hook-free check (the common case) resolves to `None` so
//! callers add no output for it.

use duhem_schema::{FlowCleanup, Step, VerificationDefinition};
use duhem_summary::LifecycleBlock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEntry {
    pub label: String,
    criterion_id: Option<String>,
    check_id: Option<String>,
    fixture_name: Option<String>,
    /// `(flow, call)` for a flow `cleanup:` registered at this scope (#615).
    registration: Option<(String, String)>,
}

impl HookEntry {
    fn new(
        label: String,
        criterion_id: Option<&str>,
        check_id: Option<&str>,
        fixture_name: Option<&str>,
    ) -> Self {
        Self {
            label,
            criterion_id: criterion_id.map(str::to_string),
            check_id: check_id.map(str::to_string),
            fixture_name: fixture_name.map(str::to_string),
            registration: None,
        }
    }

    fn registered(
        cleanup: &FlowCleanup,
        site: String,
        criterion_id: Option<&str>,
        check_id: Option<&str>,
    ) -> Self {
        Self {
            registration: Some((cleanup.flow.clone(), cleanup.call.clone())),
            ..Self::new(
                format!(
                    "flow `{}` cleanup (call `{}`, registered in {site})",
                    cleanup.flow, cleanup.call
                ),
                criterion_id,
                check_id,
                None,
            )
        }
    }

    pub fn matches(&self, block: &LifecycleBlock) -> bool {
        let registered = block
            .registered_by
            .as_ref()
            .map(|r| (r.flow.clone(), r.call.clone()));
        registered == self.registration
            && block.matches_evidence_scope(
                self.criterion_id.as_deref(),
                self.check_id.as_deref(),
                self.fixture_name.as_deref(),
            )
    }
}

/// The ordered, resolved hook chain for one check, split at the
/// check's own `steps:` (which is not itself a hook). Each entry
/// names both what runs and where it was declared — the criterion id,
/// check id, or fixture name is the declaration site.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HookChain {
    /// In execution order: leaf `setup:` → criterion `setup:` → check
    /// `setup:` → each `needs:` fixture's `up:`, in `needs:` order.
    pub before: Vec<HookEntry>,
    /// In execution order (exact reverse of `before`'s fixture/level
    /// nesting): each `needs:` fixture's `down:`, in reverse `needs:`
    /// order → check `teardown:` → criterion `teardown:` → leaf
    /// `teardown:`.
    pub after: Vec<HookEntry>,
}

impl HookChain {
    fn is_empty(&self) -> bool {
        self.before.is_empty() && self.after.is_empty()
    }
}

/// Resolve the hook chain for `criterion_id`/`check_id` in `def`.
/// `None` when the check has no hooks at all (no leaf/criterion/check
/// `setup:`/`teardown:` and no `needs:`) or when the ids don't
/// resolve (a malformed caller, not expected in practice — the ids
/// come from evidence the run itself produced).
pub fn resolve(
    def: &VerificationDefinition,
    criterion_id: &str,
    check_id: &str,
) -> Option<HookChain> {
    let criterion = def.criteria.iter().find(|c| c.id == criterion_id)?;
    let check = criterion.checks.iter().find(|c| c.id == check_id)?;

    let mut chain = HookChain::default();
    if !def.setup.is_empty() {
        chain
            .before
            .push(HookEntry::new("leaf setup".to_string(), None, None, None));
    }
    if !criterion.setup.is_empty() {
        chain.before.push(HookEntry::new(
            format!("criterion `{}` setup", criterion.id),
            Some(&criterion.id),
            None,
            None,
        ));
    }
    if !check.setup.is_empty() {
        chain.before.push(HookEntry::new(
            format!("check `{}` setup", check.id),
            Some(&criterion.id),
            Some(&check.id),
            None,
        ));
    }
    for name in &check.needs {
        chain.before.push(HookEntry::new(
            format!("fixture `{name}` up"),
            None,
            Some(&check.id),
            Some(name),
        ));
    }

    // Flow `cleanup:` unwinds LIFO at the exit of the scope that registered
    // it, before that scope's own `teardown:` (#615).
    let registered = |steps: &[&Step], site: String, c: Option<&str>, k: Option<&str>| {
        steps
            .iter()
            .flat_map(|s| s.actions())
            .flat_map(|s| s.flow_cleanup.iter())
            .map(|cleanup| HookEntry::registered(cleanup, site.clone(), c, k))
            .collect::<Vec<_>>()
    };
    let check_steps: Vec<&Step> = check.setup.iter().chain(&check.steps).collect();
    chain.after.extend(
        registered(
            &check_steps,
            format!("check `{}`", check.id),
            Some(&criterion.id),
            Some(&check.id),
        )
        .into_iter()
        .rev(),
    );

    for name in check.needs.iter().rev() {
        chain.after.push(HookEntry::new(
            format!("fixture `{name}` down"),
            None,
            Some(&check.id),
            Some(name),
        ));
    }
    if !check.teardown.is_empty() {
        chain.after.push(HookEntry::new(
            format!("check `{}` teardown", check.id),
            Some(&criterion.id),
            Some(&check.id),
            None,
        ));
    }
    chain.after.extend(
        registered(
            &criterion.setup.iter().collect::<Vec<_>>(),
            format!("criterion `{}` setup", criterion.id),
            Some(&criterion.id),
            None,
        )
        .into_iter()
        .rev(),
    );
    if !criterion.teardown.is_empty() {
        chain.after.push(HookEntry::new(
            format!("criterion `{}` teardown", criterion.id),
            Some(&criterion.id),
            None,
            None,
        ));
    }
    chain.after.extend(
        registered(
            &def.setup.iter().collect::<Vec<_>>(),
            "leaf setup".to_string(),
            None,
            None,
        )
        .into_iter()
        .rev(),
    );
    if !def.teardown.is_empty() {
        chain.after.push(HookEntry::new(
            "leaf teardown".to_string(),
            None,
            None,
            None,
        ));
    }

    if chain.is_empty() { None } else { Some(chain) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(yaml: &str) -> VerificationDefinition {
        VerificationDefinition::from_yaml_str(yaml).expect("parse")
    }

    #[test]
    fn hookless_check_resolves_to_none() {
        let v = def(r#"
verification: x
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#);
        assert_eq!(resolve(&v, "AC-1", "AC-1.1"), None);
    }

    #[test]
    fn one_hook_resolves_to_a_single_entry() {
        let v = def(r#"
verification: x
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        setup:
          - uses: cli/invoke
        assertions: ["true"]
"#);
        let chain = resolve(&v, "AC-1", "AC-1.1").expect("has a hook");
        assert_eq!(chain.before[0].label, "check `AC-1.1` setup");
        assert_eq!(chain.before.len(), 1);
        assert!(chain.after.is_empty());
    }

    #[test]
    fn all_four_levels_plus_fixtures_resolve_in_documented_order() {
        let v = def(r#"
verification: x
setup: [{ uses: cli/invoke }]
teardown: [{ uses: cli/invoke }]
fixtures:
  res:
    up: [{ uses: cli/invoke }]
    down: [{ uses: cli/invoke }]
criteria:
  - id: AC-1
    description: x
    setup: [{ uses: cli/invoke }]
    teardown: [{ uses: cli/invoke }]
    checks:
      - id: AC-1.1
        needs: [res]
        setup: [{ uses: cli/invoke }]
        teardown: [{ uses: cli/invoke }]
        assertions: ["true"]
"#);
        let chain = resolve(&v, "AC-1", "AC-1.1").expect("has hooks");
        assert_eq!(
            chain
                .before
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "leaf setup",
                "criterion `AC-1` setup",
                "check `AC-1.1` setup",
                "fixture `res` up",
            ]
        );
        assert_eq!(
            chain
                .after
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "fixture `res` down",
                "check `AC-1.1` teardown",
                "criterion `AC-1` teardown",
                "leaf teardown",
            ]
        );
    }

    #[test]
    fn unknown_ids_resolve_to_none() {
        let v = def(r#"
verification: x
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#);
        assert_eq!(resolve(&v, "AC-9", "AC-1.1"), None);
        assert_eq!(resolve(&v, "AC-1", "AC-9.9"), None);
    }

    #[test]
    fn registered_flow_cleanups_are_listed_lifo_with_their_registration_site() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("leaf.yml");
        std::fs::write(
            &path,
            r#"
verification: x
flows:
  res:
    params: { name: { type: string } }
    steps: [{ id: make, uses: cli/invoke }]
    cleanup: [{ uses: cli/invoke }]
setup: [{ id: leaf, call: res, with: { name: l } }]
teardown: [{ uses: cli/invoke }]
criteria:
  - id: AC-1
    description: x
    setup: [{ id: crit, call: res, with: { name: c } }]
    checks:
      - id: AC-1.1
        steps:
          - { id: one, call: res, with: { name: a } }
          - { id: two, call: res, with: { name: b } }
        teardown: [{ uses: cli/invoke }]
        assertions: ["true"]
"#,
        )
        .unwrap();
        let duhem_schema::Loaded::Leaf { definition, .. } = duhem_schema::load(&path).unwrap()
        else {
            panic!("expected a leaf");
        };
        let chain = resolve(&definition, "AC-1", "AC-1.1").expect("has hooks");
        let labels: Vec<_> = chain.after.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "flow `res` cleanup (call `two`, registered in check `AC-1.1`)",
                "flow `res` cleanup (call `one`, registered in check `AC-1.1`)",
                "check `AC-1.1` teardown",
                "flow `res` cleanup (call `crit`, registered in criterion `AC-1` setup)",
                "flow `res` cleanup (call `leaf`, registered in leaf setup)",
                "leaf teardown",
            ]
        );
        let block = |registered_by: Option<(&str, &str)>| LifecycleBlock {
            cleanup_policy: Some("required".into()),
            registered_by: registered_by.map(|(flow, call)| duhem_summary::LifecycleRegistration {
                flow: flow.into(),
                call: call.into(),
            }),
            phase: duhem_summary::LifecyclePhase::Teardown,
            scope: vec![
                duhem_summary::LifecycleScopeSegment {
                    kind: "criterion".into(),
                    id: "AC-1".into(),
                },
                duhem_summary::LifecycleScopeSegment {
                    kind: "check".into(),
                    id: "AC-1.1".into(),
                },
            ],
            status: duhem_summary::LifecycleStatus::Passed,
            started_at: "t".into(),
            duration_ms: 0,
            steps: vec![],
            failing_step: None,
        };
        assert!(chain.after[0].matches(&block(Some(("res", "two")))));
        assert!(!chain.after[0].matches(&block(Some(("res", "one")))));
        assert!(!chain.after[0].matches(&block(None)));
        assert!(
            chain.after[2].matches(&block(None)),
            "a teardown entry still matches a plain block"
        );
        assert!(!chain.after[2].matches(&block(Some(("res", "two")))));
    }
}
