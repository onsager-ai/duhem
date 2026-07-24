//! Versioned browser-session evidence for synchronized dashboard replay.
//!
//! The document is carried as a runner-emitted `capture/session-evidence`
//! blob observation. Keeping it behind the existing observation/blob seam
//! makes the addition backward compatible with v1 event readers while still
//! giving all replay facts one explicitly versioned contract.

use serde::{Deserialize, Serialize};

pub const SESSION_EVIDENCE_VERSION: u32 = 1;
pub const SESSION_EVIDENCE_OBSERVATION: &str = "capture/session-evidence";
pub const STEP_SCREENSHOT_OBSERVATION: &str = "capture/step-screenshot";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvidence {
    pub version: u32,
    /// Always `monotonic` in v1. All values are milliseconds from the
    /// check browser context's origin.
    pub clock: String,
    pub duration_ms: f64,
    pub steps: Vec<SessionStep>,
    #[serde(default)]
    pub network: Vec<SessionNetworkEntry>,
    #[serde(default)]
    pub performance: Vec<SessionPerformanceObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<SessionVideo>,
}

impl SessionEvidence {
    pub fn v1(duration_ms: f64) -> Self {
        Self {
            version: SESSION_EVIDENCE_VERSION,
            clock: "monotonic".to_string(),
            duration_ms,
            steps: Vec::new(),
            network: Vec::new(),
            performance: Vec::new(),
            video: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionStep {
    pub step_index: u32,
    pub started_ms: f64,
    pub finished_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionNetworkEntry {
    pub method: String,
    pub url: String,
    pub status: u16,
    pub started_ms: f64,
    pub duration_ms: f64,
    pub wait_ms: f64,
    pub receive_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionPerformanceObservation {
    /// `navigation`, `resource`, `paint`, `web-vital`, or `long-task`.
    pub kind: String,
    pub name: String,
    pub started_ms: f64,
    pub duration_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionVideo {
    pub started_ms: f64,
    pub finished_ms: f64,
    pub blob_sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_document_round_trips_and_defaults_additive_collections() {
        let json = r#"{"version":1,"clock":"monotonic","duration_ms":12.5,"steps":[]}"#;
        let parsed: SessionEvidence = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, SessionEvidence::v1(12.5));
        let encoded = serde_json::to_string(&parsed).unwrap();
        assert_eq!(
            serde_json::from_str::<SessionEvidence>(&encoded).unwrap(),
            parsed
        );
    }
}
