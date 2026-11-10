//! Page-error and console records from the sidecar's bounded per-page
//! recorder (`ui/observe-console`, #603 P4).

use serde_json::json;

use super::{Page, PwError};

impl Page {
    /// Read the bounded page error and console recorder.
    pub async fn console_records(&self) -> Result<serde_json::Value, PwError> {
        self.conn.request("consoleRecords", self.p()).await
    }

    /// Lower the recorder cap for browser tests.
    #[doc(hidden)]
    pub async fn set_console_cap_for_tests(&self, cap: u64) -> Result<(), PwError> {
        let mut params = self.p();
        params["cap"] = json!(cap);
        self.conn.request("setConsoleCap", params).await?;
        Ok(())
    }
}
