//! Allocate a context and its page as one resource lifecycle.
use super::*;

impl RunBrowser {
    pub(super) async fn open_check_inner(
        &self,
        storage_state: Option<&serde_json::Value>,
    ) -> Result<CheckBrowser, ActionError> {
        let viewport = self.viewport.map_or(
            serde_json::Value::Null,
            |v| json!({ "width": v.width, "height": v.height }),
        );
        let params = match storage_state {
            Some(state) => {
                json!({ "recordVideo": self.record_video, "storageState": state, "viewport": viewport })
            }
            None => json!({ "recordVideo": self.record_video, "viewport": viewport }),
        };
        let ctx = self
            .conn
            .request("newContext", params)
            .await
            .map_err(|e| ActionError::Playwright(format!("context: {e}")))?;
        let context_id = ctx
            .get("contextId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ActionError::Playwright("newContext: missing contextId".into()))?
            .to_string();

        let page_id = async {
            let pg = self
                .conn
                .request("newPage", json!({ "contextId": context_id }))
                .await
                .map_err(|e| ActionError::Playwright(format!("page: {e}")))?;
            pg.get("pageId")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| ActionError::Playwright("newPage: missing pageId".into()))
        }
        .await;
        let page_id = match page_id {
            Ok(id) => id,
            Err(error) => {
                // A failed page allocation must not strand its already-open context.
                let _ = self
                    .conn
                    .request(
                        "closeContext",
                        json!({
                            "contextId": context_id, "keepVideo": false, "maxBytes": 0,
                        }),
                    )
                    .await;
                return Err(error);
            }
        };

        Ok(CheckBrowser {
            conn: self.conn.clone(),
            context_id,
            page: Page {
                conn: self.conn.clone(),
                id: page_id,
            },
        })
    }
}
