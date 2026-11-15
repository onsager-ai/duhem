//! Optional plan metadata shared by evidence and summaries (#650).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardIdentity {
    /// One-based, like Playwright's `--shard=1/3`.
    pub index: usize,
    pub total: usize,
    /// `flag`, `env`, `gitlab`, `buildkite`, or `circleci`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedPartition {
    pub units: Vec<String>,
    pub total_units: usize,
    /// Selected units assigned to another machine, including native commands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<ExcludedUnit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludedUnit {
    pub unit: String,
    pub reason: String,
}

/// Flattened into JSON summaries; all fields are absent on ordinary runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributedRun {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shard: Option<ShardIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_partition: Option<ExpectedPartition>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordinary_summary_bytes_stay_frozen_and_old_summaries_load() {
        let expected = r#"{"schema_version":"2","run_id":"fixed","verdict":"pass","criteria":[],"store":"evidence.db","totals":{"total":0,"passed":0,"failed":0,"inconclusive":0},"failures":[]}"#;
        let summary = crate::RunSummary::new(
            "fixed",
            duhem_judge::VerdictState::Pass,
            Vec::new(),
            "evidence.db".into(),
        );
        assert_eq!(serde_json::to_string(&summary).unwrap(), expected);
        let restored: crate::RunSummary = serde_json::from_str(expected).unwrap();
        assert_eq!(restored, summary);
    }
}
