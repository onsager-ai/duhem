//! Failure-evidence capture (spec #202).
//!
//! The runner records a full-page screenshot + DOM snapshot of a
//! `ui/*` check when it ends non-pass (or, under `always`, on every
//! ui check). Captures ride the existing #10 `step_observation` blob
//! channel under the reserved `capture/` output-name prefix, so the
//! dashboard/hub artifact pipeline renders them with no read-side
//! change. Captures are evidence for humans and agents, never judge
//! input; a capture failure warns and never touches the verdict.

use std::time::Duration;

use duhem_actions::Page;
use duhem_evidence::{EventPayload, EvidenceWriter, ObservationValue};
use tracing::warn;

use crate::engine::har;
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

/// Hard wall-clock ceiling on each capture op. The screenshot's
/// `CAPTURE_TIMEOUT_MS` is a Playwright-side deadline — it can't fire
/// if the sidecar pipe itself wedges (the request never round-trips),
/// and `page.dom()` has no browser-side timeout at all. This bounds
/// teardown regardless of where the stall is. Slightly above
/// `CAPTURE_TIMEOUT_MS` so a real browser-side timeout surfaces as
/// its own error rather than racing this one.
const CAPTURE_DEADLINE: Duration = Duration::from_millis(12_000);

/// Reserved output-name prefix for runner-emitted captures. Enforced
/// at authoring time (`duhem-schema` rejects an authored output alias
/// under `capture/`) and never produced by any action, so the runtime
/// is the only source of `capture/*` evidence. Captures are not
/// recorded as `$steps.<id>.outputs.*` bindings, so assertions can't
/// reference them either.
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
    match bounded(
        "screenshot",
        CAPTURE_DEADLINE,
        page.screenshot(CAPTURE_TIMEOUT_MS),
    )
    .await
    {
        Some(Ok(png)) => {
            if let Some(c) = append_capture(writer, step_index, CAPTURE_SCREENSHOT, &png).await {
                captured.push(c);
            }
        }
        Some(Err(e)) => warn!(error = %e, "screenshot capture failed; verdict unaffected"),
        None => {}
    }
    match bounded("dom", CAPTURE_DEADLINE, page.dom()).await {
        Some(Ok(html)) => {
            if let Some(c) = append_capture(writer, step_index, CAPTURE_DOM, html.as_bytes()).await
            {
                captured.push(c);
            }
        }
        Some(Err(e)) => warn!(error = %e, "dom capture failed; verdict unaffected"),
        None => {}
    }
    // Network HAR tail (spec #204): the browser page's recorded traffic
    // — the network the delivery web generated as the UI drove it. The
    // recorder always ran, so we drain the whole buffer and keep the
    // tail. An empty buffer (page-free check, or no requests) emits
    // nothing. Serialization redacts secrets before the blob is stored.
    match bounded("network", CAPTURE_DEADLINE, page.poll_network(0)).await {
        Some(Ok(batch)) if !batch.events.is_empty() => {
            let har = har::to_har(&batch.events, har::DEFAULT_BODY_CAP);
            if let Some(c) =
                append_capture(writer, step_index, har::CAPTURE_NETWORK, har.as_bytes()).await
            {
                captured.push(c);
            }
        }
        Some(Ok(_)) => {}
        Some(Err(e)) => warn!(error = %e, "network capture failed; verdict unaffected"),
        None => {}
    }
    captured
}

/// Run one capture op under `deadline`. `None` on timeout (logged) —
/// a wedged sidecar pipe can't stall the run's teardown.
async fn bounded<T>(
    op: &str,
    deadline: Duration,
    fut: impl std::future::Future<Output = T>,
) -> Option<T> {
    match tokio::time::timeout(deadline, fut).await {
        Ok(v) => Some(v),
        Err(_) => {
            warn!(op, "capture op timed out; verdict unaffected");
            None
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_returns_none_when_the_op_outlives_the_deadline() {
        // A capture op that never resolves within the deadline yields
        // `None` (teardown proceeds) rather than hanging the run. A
        // tiny real deadline against a long sleep keeps the test fast.
        let wedged = async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            "never observed"
        };
        assert_eq!(
            bounded("dom", Duration::from_millis(10), wedged).await,
            None
        );
    }

    #[tokio::test]
    async fn bounded_passes_a_prompt_result_through() {
        assert_eq!(
            bounded("screenshot", CAPTURE_DEADLINE, async { 42 }).await,
            Some(42)
        );
    }
}
