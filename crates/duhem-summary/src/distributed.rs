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
