//! Authored, selected and recorded claim populations are distinct.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimCounts {
    pub authored: u32,
    pub selected: u32,
    /// Distinct claims with recorded outcomes, never the number of attempts.
    /// An inconclusive precondition outcome is not evidence of a passing body.
    pub executed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExcludedClaim {
    pub criterion_id: String,
    pub check_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExcludedLeaf {
    pub verification: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestSelection {
    pub authored: u32,
    pub selected: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<ExcludedLeaf>,
}

impl std::fmt::Display for ManifestSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} of {} leaves selected", self.selected, self.authored)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimScope {
    pub criteria: ClaimCounts,
    pub checks: ClaimCounts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<ExcludedClaim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<ManifestSelection>,
}

impl std::fmt::Display for ClaimScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} of {} criteria, {} of {} checks recorded ({} criteria, {} checks selected)",
            self.criteria.executed,
            self.criteria.authored,
            self.checks.executed,
            self.checks.authored,
            self.criteria.selected,
            self.checks.selected,
        )?;
        if let Some(manifest) = &self.manifest {
            write!(f, "; {manifest}")?;
        }
        Ok(())
    }
}
