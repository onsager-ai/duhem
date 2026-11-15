//! Gate-only results from a project's own verification commands.
use duhem_judge::VerdictState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeRunSummary {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    pub outcome: VerdictState,
    pub duration_ms: u64,
    pub required: bool,
    pub tests: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<NativeTestCounts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_blob_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results_blob_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failing_tests: Vec<NativeTestFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_blob_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_blob_sha256: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeTestCounts {
    pub passed: u32,
    pub failed: u32,
    pub errored: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeTestFailure {
    pub test_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerTotals {
    pub layer: String,
    pub tests: u64,
    pub passed: u64,
    pub failed: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub errored: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub skipped: u64,
    pub inconclusive: u64,
    pub duration_ms: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

pub fn layer_totals(
    native: &[NativeRunSummary],
    checks: super::CheckTotals,
    check_duration_ms: u64,
) -> Vec<LayerTotals> {
    let mut totals = std::collections::BTreeMap::<String, LayerTotals>::new();
    totals.insert(
        "duhem".into(),
        LayerTotals {
            layer: "duhem".into(),
            tests: checks.total.into(),
            passed: checks.passed.into(),
            failed: checks.failed.into(),
            inconclusive: checks.inconclusive.into(),
            duration_ms: check_duration_ms,
            ..Default::default()
        },
    );
    for run in native {
        let layer = run.layer.as_deref().unwrap_or("other").to_string();
        let row = totals.entry(layer.clone()).or_insert_with(|| LayerTotals {
            layer,
            ..Default::default()
        });
        row.tests += u64::from(run.tests);
        row.duration_ms += run.duration_ms;
        if let Some(counts) = &run.counts {
            row.passed += u64::from(counts.passed);
            row.failed += u64::from(counts.failed);
            row.errored += u64::from(counts.errored);
            row.skipped += u64::from(counts.skipped);
            continue;
        }
        match run.outcome {
            VerdictState::Pass => row.passed += u64::from(run.tests),
            VerdictState::Fail => row.failed += u64::from(run.tests),
            VerdictState::Inconclusive(_) => row.inconclusive += u64::from(run.tests),
        }
    }
    totals.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckTotals, RunSetSummary, RunSummary};

    #[test]
    fn absent_native_preserves_existing_json_bytes() {
        let summary = RunSummary::new("r", VerdictState::Pass, vec![], "store.db".into());
        assert_eq!(
            serde_json::to_string(&summary).unwrap(),
            r#"{"schema_version":"2","run_id":"r","verdict":"pass","criteria":[],"store":"store.db","totals":{"total":0,"passed":0,"failed":0,"inconclusive":0},"failures":[]}"#
        );
        let set = RunSetSummary::new("s", VerdictState::Pass, vec![]);
        assert_eq!(
            serde_json::to_string(&set).unwrap(),
            r#"{"run_id":"s","schema_version":"1","verdict":"pass","totals":{"total":0,"passed":0,"failed":0,"inconclusive":0},"runs":[]}"#
        );
    }

    #[test]
    fn exit_code_native_json_stays_byte_identical() {
        let json = r#"{"id":"lint","outcome":"pass","duration_ms":12,"required":true,"tests":1,"exit_code":0}"#;
        let native: NativeRunSummary = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&native).unwrap(), json);
        let layer = r#"{"layer":"static","tests":1,"passed":1,"failed":0,"inconclusive":0,"duration_ms":12}"#;
        let totals: LayerTotals = serde_json::from_str(layer).unwrap();
        assert_eq!(serde_json::to_string(&totals).unwrap(), layer);
    }

    #[test]
    fn child_native_counts_and_durations_are_aggregated() {
        let command = NativeRunSummary {
            id: "unit".into(),
            layer: Some("unit".into()),
            outcome: VerdictState::Fail,
            duration_ms: 12,
            required: false,
            tests: 1,
            counts: None,
            report_path: None,
            report_blob_sha256: None,
            results_blob_sha256: None,
            failing_tests: vec![],
            exit_code: Some(1),
            detail: Some("failed".into()),
            stdout_blob_sha256: None,
            stderr_blob_sha256: None,
        };
        let child = RunSummary::new("r", VerdictState::Pass, vec![], "store.db".into())
            .with_totals(CheckTotals {
                total: 2,
                passed: 2,
                ..Default::default()
            })
            .with_native(vec![command.clone()], 25);
        let set = RunSetSummary::new("s", VerdictState::Pass, vec![child.clone(), child]);
        assert_eq!(set.native, vec![command.clone(), command]);
        assert_eq!(set.layers.iter().map(|row| row.tests).sum::<u64>(), 6);
        assert_eq!(
            set.layers.iter().map(|row| row.duration_ms).sum::<u64>(),
            74
        );
        assert_eq!(
            set.layers
                .iter()
                .find(|row| row.layer == "unit")
                .unwrap()
                .failed,
            2
        );
    }
}
