//! Request-start records from the sidecar's bounded per-page recorder
//! (`api/observe` windowed mode, #603 P3).

use serde::Deserialize;
use serde_json::json;

use super::{Page, PwError};

/// Request-start records from the page's bounded network recorder.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct RequestRecord {
    pub seq: u64,
    pub method: String,
    pub url: String,
    pub started_ms: f64,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirected_from: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RequestRecords {
    pub events: Vec<RequestRecord>,
    pub truncated: bool,
    pub attached_ms: f64,
}

impl Page {
    /// Read the bounded request-start recorder for this page.
    pub async fn request_records(&self) -> Result<RequestRecords, PwError> {
        let v = self.conn.request("requestRecords", self.p()).await?;
        serde_json::from_value(v).map_err(|e| PwError(format!("requestRecords decode: {e}")))
    }

    /// Test-only: lower this page's request-record cap (default 10 000)
    /// so overflow behavior can be exercised without thousands of requests.
    #[doc(hidden)]
    pub async fn set_request_cap_for_tests(&self, cap: u64) -> Result<(), PwError> {
        let mut params = self.p();
        params["cap"] = json!(cap);
        self.conn.request("setRequestCap", params).await?;
        Ok(())
    }
}
