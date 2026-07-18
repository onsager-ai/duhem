//! `Criterion` and `Check` — the criteria-vs-checks separation made
//! structural.
//!
//! `Criterion.description` is opaque prose: the human commitment about
//! what "done" means (`docs/duhem-spec.md` §7.2). The schema never
//! introspects it.
//!
//! `Check` carries no back-reference to "which version of the
//! criterion produced me" — checks are derivative (§7.3) and may be
//! regenerated as the implementation evolves; round-tripping authored
//! YAML order keeps regeneration diffs reviewable.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::assertion::Assertion;
use crate::step::Step;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    /// Authored stable identifier (e.g. `AC-1`). Required and authored
    /// — auto-generation hides intent and breaks evidence-trace
    /// stability across runs.
    pub id: String,

    /// Free-form prose. Opaque to the schema layer.
    pub description: String,

    /// One or more checks that, taken together, verify this criterion.
    /// The judge's per-criterion verdict is an aggregation of the
    /// per-check verdicts.
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// Authored stable identifier (e.g. `AC-1.1`).
    pub id: String,

    /// Optional human-readable summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Ordered sequence of action invocations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,

    /// Mechanically-judgable claims about what the steps must produce.
    /// Optional since spec #253: a check may rely entirely on the
    /// implicit judgment of its judging steps (actions whose contract
    /// emits a boolean `satisfied` output). A check with neither
    /// assertions nor steps has nothing to judge — the validator
    /// rejects it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assertions: Vec<Assertion>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_criterion() {
        let y = r#"
id: AC-1
description: A user can create a workspace.
checks:
  - id: AC-1.1
    steps: []
    assertions:
      - $inputs.x == 1
"#;
        let c: Criterion = serde_yml::from_str(y).expect("parse");
        assert_eq!(c.id, "AC-1");
        assert_eq!(c.checks.len(), 1);
        assert_eq!(c.checks[0].assertions.len(), 1);
    }

    #[test]
    fn parses_check_without_assertions() {
        // Spec #253: `assertions:` may be omitted when a judging step
        // carries the verdict; emptiness rules live in the validator.
        let y = r#"
id: AC-1
description: x
checks:
  - id: AC-1.1
    steps:
      - uses: ui/assert-element
        with: { locator: { role: button, name: Save }, expected: visible }
"#;
        let c: Criterion = serde_yml::from_str(y).expect("parse");
        assert!(c.checks[0].assertions.is_empty());
        // Round-trip: an empty assertions list is not serialized.
        let out = serde_yml::to_string(&c).expect("serialize");
        assert!(!out.contains("assertions"), "got: {out}");
    }

    #[test]
    fn rejects_check_missing_id() {
        let y = r#"
id: AC-1
description: x
checks:
  - steps: []
    assertions: []
"#;
        assert!(serde_yml::from_str::<Criterion>(y).is_err());
    }

    #[test]
    fn rejects_unknown_field_on_check() {
        let y = r#"
id: AC-1
description: x
checks:
  - id: AC-1.1
    foo: bar
    steps: []
    assertions: []
"#;
        let err = serde_yml::from_str::<Criterion>(y).unwrap_err();
        assert!(format!("{err}").contains("unknown field"), "got: {err}");
    }
}
