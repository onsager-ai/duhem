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

use duhem_schema::VerificationDefinition;

/// The ordered, resolved hook chain for one check, split at the
/// check's own `steps:` (which is not itself a hook). Each entry
/// names both what runs and where it was declared — the criterion id,
/// check id, or fixture name is the declaration site.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HookChain {
    /// In execution order: leaf `setup:` → criterion `setup:` → check
    /// `setup:` → each `needs:` fixture's `up:`, in `needs:` order.
    pub before: Vec<String>,
    /// In execution order (exact reverse of `before`'s fixture/level
    /// nesting): each `needs:` fixture's `down:`, in reverse `needs:`
    /// order → check `teardown:` → criterion `teardown:` → leaf
    /// `teardown:`.
    pub after: Vec<String>,
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
        chain.before.push("leaf setup".to_string());
    }
    if !criterion.setup.is_empty() {
        chain
            .before
            .push(format!("criterion `{}` setup", criterion.id));
    }
    if !check.setup.is_empty() {
        chain.before.push(format!("check `{}` setup", check.id));
    }
    for name in &check.needs {
        chain.before.push(format!("fixture `{name}` up"));
    }

    for name in check.needs.iter().rev() {
        chain.after.push(format!("fixture `{name}` down"));
    }
    if !check.teardown.is_empty() {
        chain.after.push(format!("check `{}` teardown", check.id));
    }
    if !criterion.teardown.is_empty() {
        chain
            .after
            .push(format!("criterion `{}` teardown", criterion.id));
    }
    if !def.teardown.is_empty() {
        chain.after.push("leaf teardown".to_string());
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
        assert_eq!(chain.before, vec!["check `AC-1.1` setup".to_string()]);
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
            chain.before,
            vec![
                "leaf setup".to_string(),
                "criterion `AC-1` setup".to_string(),
                "check `AC-1.1` setup".to_string(),
                "fixture `res` up".to_string(),
            ]
        );
        assert_eq!(
            chain.after,
            vec![
                "fixture `res` down".to_string(),
                "check `AC-1.1` teardown".to_string(),
                "criterion `AC-1` teardown".to_string(),
                "leaf teardown".to_string(),
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
}
