//! Browser-context emulation for `ui/emulate` (#580).
use super::*;

/// Effective emulation settings as observed on the page.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EffectiveEmulation {
    pub viewport: EffectiveViewport,
    #[serde(rename = "colorScheme")]
    pub color_scheme: String,
}

/// `window.innerWidth/innerHeight` as the page reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct EffectiveViewport {
    pub width: u32,
    pub height: u32,
}

impl Page {
    /// Apply the requested viewport and/or color scheme to this page and
    /// return what the page then reports (`innerWidth/innerHeight` and
    /// `prefers-color-scheme` media queries), not the requested values.
    pub async fn emulate(
        &self,
        viewport: Option<duhem_schema::Viewport>,
        color_scheme: Option<&str>,
    ) -> Result<EffectiveEmulation, PwError> {
        let mut req = self.p();
        if let Some(v) = viewport {
            req["viewport"] = json!({ "width": v.width, "height": v.height });
        }
        if let Some(scheme) = color_scheme {
            req["colorScheme"] = json!(scheme);
        }
        let v = self.conn.request("emulate", req).await?;
        serde_json::from_value(v).map_err(|e| PwError(format!("emulate result decode: {e}")))
    }
}
