//! Failure-evidence capture (spec #202).
//!
//! The runner records a full-page screenshot + DOM snapshot of a
//! `ui/*` check when it ends non-pass (or, under `always`, on every
//! ui check). Captures ride the existing #10 `step_observation` blob
//! channel under the reserved `capture/` output-name prefix, so the
//! dashboard/hub artifact pipeline renders them with no read-side
//! change. Captures are evidence for humans and agents, never judge
//! input; a capture failure warns and never touches the verdict.

use duhem_actions::Page;
use duhem_evidence::{EventPayload, EvidenceWriter, ObservationValue};
use tracing::warn;

use crate::engine::outcome::CapturedArtifact;

/// Failure-evidence capture policy. A runner knob, not an authored
/// contract: `on-failure` (the default) captures when a ui check ends
/// with any non-pass assertion; `always` also captures the final
/// state of passing ui checks; `off` disables capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CapturePolicy {
    #[default]
    OnFailure,
    Always,
    Off,
}

impl std::str::FromStr for CapturePolicy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "on-failure" => Ok(Self::OnFailure),
            "always" => Ok(Self::Always),
            "off" => Ok(Self::Off),
            other => Err(format!(
                "unknown capture policy `{other}` (expected on-failure | always | off)"
            )),
        }
    }
}

/// Ceiling for one capture op. Generous because a full-page
/// screenshot on a heavy page is slow, but bounded so a wedged page
/// can't stall the run's teardown.
const CAPTURE_TIMEOUT_MS: f64 = 10_000.0;

/// Reserved output-name prefix for runner-emitted captures. Authored
/// step outputs cannot contain `/` in practice and the prefix is
/// documented as reserved, so assertions never bind to captures.
const CAPTURE_SCREENSHOT: &str = "capture/screenshot";
const CAPTURE_DOM: &str = "capture/dom";

/// Capture a full-page screenshot + DOM snapshot and append them as
/// `capture/*` blob observations. Returns the refs that actually
/// landed so the reporter can point at them. Every failure in here is
/// a warning, never an error: evidence gathering must not mask or
/// manufacture a verdict.
pub(crate) async fn capture_failure_evidence(
    writer: &mut EvidenceWriter,
    page: &Page,
    step_index: u32,
) -> Vec<CapturedArtifact> {
    let mut captured = Vec::new();
    match page.screenshot(CAPTURE_TIMEOUT_MS).await {
        Ok(png) => {
            if let Some(c) = append_capture(writer, step_index, CAPTURE_SCREENSHOT, &png).await {
                captured.push(c);
            }
        }
        Err(e) => warn!(error = %e, "screenshot capture failed; verdict unaffected"),
    }
    match page.dom().await {
        Ok(html) => {
            if let Some(c) = append_capture(writer, step_index, CAPTURE_DOM, html.as_bytes()).await
            {
                captured.push(c);
            }
        }
        Err(e) => warn!(error = %e, "dom capture failed; verdict unaffected"),
    }
    captured
}

async fn append_capture(
    writer: &mut EvidenceWriter,
    step_index: u32,
    name: &str,
    bytes: &[u8],
) -> Option<CapturedArtifact> {
    let sha = match writer.write_blob(bytes).await {
        Ok(sha) => sha,
        Err(e) => {
            warn!(error = %e, capture = name, "capture blob write failed; verdict unaffected");
            return None;
        }
    };
    if let Err(e) = writer
        .append(EventPayload::StepObservation {
            step_index,
            output_name: name.to_string(),
            value: ObservationValue::Blob {
                blob_sha256: sha.0.clone(),
            },
        })
        .await
    {
        warn!(error = %e, capture = name, "capture observation append failed; verdict unaffected");
        return None;
    }
    Some(CapturedArtifact {
        kind: name.to_string(),
        sha256: sha.0,
    })
}
