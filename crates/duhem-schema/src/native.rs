//! Project-owned verification commands. Results gate a run, never criteria.
use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::DurationSpec;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeRun {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<NativeLayer>,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<DurationSpec>,
    #[serde(default = "required", skip_serializing_if = "is_required")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<NativeExpect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<NativeReport>,
}

fn required() -> bool {
    true
}
fn is_required(value: &bool) -> bool {
    *value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeLayer {
    Unit,
    Integration,
    E2e,
    Static,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeReport {
    pub format: NativeReportFormat,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeReportFormat {
    Junit,
    Ctrf,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeExpect {
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<NativeOutputExpect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<NativeOutputExpect>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeOutputExpect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_matches: Option<String>,
}

pub fn validate_native(runs: &[NativeRun]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for run in runs {
        if run.id.trim().is_empty() {
            return Err("native run id must not be empty".into());
        }
        if !ids.insert(&run.id) {
            return Err(format!("duplicate native run id `{}`", run.id));
        }
        if run.command.is_empty() || run.command[0].is_empty() {
            return Err(format!(
                "native run `{}` needs a non-empty command argv",
                run.id
            ));
        }
        if let Some(report) = &run.report
            && (report.path.trim().is_empty() || std::path::Path::new(&report.path).is_absolute())
        {
            return Err(format!(
                "native run `{}`: report path must be nonempty and relative to cwd",
                run.id
            ));
        }
        if let Some(expect) = &run.expect {
            for output in [&expect.stdout, &expect.stderr].into_iter().flatten() {
                for pattern in [&output.matches, &output.not_matches].into_iter().flatten() {
                    regex::Regex::new(pattern).map_err(|e| {
                        format!("native run `{}`: invalid expect regex: {e}", run.id)
                    })?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(src: &str) -> Result<Vec<NativeRun>, serde_yml::Error> {
        serde_yml::from_str(src)
    }
    #[test]
    fn valid_and_invalid_entries() {
        let runs = parse("- {id: lint, layer: static, command: [sh, -c, 'exit 0'], expect: {stderr: {not_matches: warning}}}").unwrap();
        assert!(runs[0].required);
        validate_native(&runs).unwrap();
        let duplicated = [runs[0].clone(), runs[0].clone()];
        assert!(
            validate_native(&duplicated)
                .unwrap_err()
                .contains("duplicate")
        );
        for invalid in [
            "- {id: x, command: ['true'], layer: unknown}",
            "- {id: x, command: ['true'], report: {format: unknown, path: x}}",
            "- {id: x, command: ['true'], report: {format: junit}}",
        ] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
        let report =
            parse("- {id: x, command: ['true'], report: {format: junit, path: report.xml}}")
                .unwrap();
        validate_native(&report).unwrap();
        let ctrf = parse("- {id: x, command: ['true'], report: {format: ctrf, path: report.json}}")
            .unwrap();
        validate_native(&ctrf).unwrap();
        for path in ["''", "/absolute.xml"] {
            let invalid = parse(&format!(
                "- {{id: x, command: ['true'], report: {{format: junit, path: {path}}}}}"
            ))
            .unwrap();
            assert!(validate_native(&invalid).is_err());
        }
    }
    #[test]
    fn native_is_not_an_expression_root() {
        let definition = crate::VerificationDefinition::from_yaml_str("verification: isolated\nnative: [{id: lint, command: ['true']}]\ncriteria:\n  - id: AC\n    description: Independent\n    checks:\n      - id: body\n        assertions: ['$native.lint.outcome == 0']\n").unwrap();
        let errors = crate::validate(&definition).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("native"))
        );
    }
}
