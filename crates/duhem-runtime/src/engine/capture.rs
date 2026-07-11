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

use duhem_actions::{CheckBrowser, Page, Rect};
use duhem_evidence::{EventPayload, EvidenceWriter, ObservationValue};
use tracing::{debug, warn};

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
const CAPTURE_TARGET_RECT: &str = "capture/target-rect";
const CAPTURE_VIDEO: &str = "capture/video";

/// Upper bound on a kept video blob (#215). Recording is opt-in
/// (`--capture-video`) and the video ships to the hosted hub, so a
/// pathologically long check can't balloon the evidence store. Over
/// the cap → warn + skip (evidence, never a verdict input).
const VIDEO_MAX_BYTES: usize = 25 * 1024 * 1024;

/// Per-locator ceiling for the bounding-box probe. Short: at capture
/// time the page state is settled, so an absent target should report
/// `found: false` quickly rather than wait out a long timeout.
const TARGET_RECT_TIMEOUT_MS: f64 = 800.0;

/// Wall-clock deadline per target-rect probe — kept just above the
/// Playwright-side timeout so a wedged sidecar can't extend teardown
/// by the full `CAPTURE_DEADLINE` once per assert-element step.
const TARGET_RECT_DEADLINE: Duration = Duration::from_millis(1_500);

/// A ui/assert-element target to probe for the element-highlight
/// overlay (spec #214): the resolved Playwright selector + the
/// authored `expected:` state.
pub(crate) struct TargetLocator {
    pub selector: String,
    pub expected: String,
}

/// The element-highlight target of a `ui/assert-element` step — its
/// resolved Playwright selector + `expected:` state. `None` for any
/// other step, or a locator that won't parse.
pub(crate) fn target_from_step(
    uses: &str,
    resolved_with: &serde_yml::Value,
) -> Option<TargetLocator> {
    if uses != "ui/assert-element" {
        return None;
    }
    let loc = resolved_with.get("locator")?;
    let locator = serde_yml::from_value::<duhem_actions::Locator>(loc.clone()).ok()?;
    Some(TargetLocator {
        selector: duhem_actions::to_selector(&locator),
        expected: resolved_with
            .get("expected")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// Record the bounding rects of a check's `ui/assert-element` targets
/// as one `capture/target-rect` JSON blob (spec #214) — the dashboard
/// overlays them on the screenshot ("we looked here"). An absent /
/// invisible target is recorded `found: false` (never a guessed box),
/// so a not-found note can replace the box. Warn-never-fail.
pub(crate) async fn capture_target_rects(
    writer: &mut EvidenceWriter,
    page: &Page,
    step_index: u32,
    targets: &[TargetLocator],
) -> Option<CapturedArtifact> {
    if targets.is_empty() {
        return None;
    }
    let mut entries = Vec::with_capacity(targets.len());
    for t in targets {
        let rect: Option<Rect> = match bounded(
            "target-rect",
            TARGET_RECT_DEADLINE,
            page.bounding_box(&t.selector, TARGET_RECT_TIMEOUT_MS),
        )
        .await
        {
            Some(Ok(r)) => r,
            Some(Err(e)) => {
                warn!(error = %e, "target-rect probe failed; verdict unaffected");
                None
            }
            None => None,
        };
        entries.push(serde_json::json!({
            "selector": t.selector,
            "expected": t.expected,
            "found": rect.is_some(),
            "rect": rect,
        }));
    }
    // Warn + skip rather than emit an empty/partial blob that would
    // mislead the dashboard (e.g. a non-finite float defeats JSON).
    match serde_json::to_vec(&entries) {
        Ok(blob) => append_capture(writer, step_index, CAPTURE_TARGET_RECT, &blob).await,
        Err(e) => {
            warn!(error = %e, "target-rect serialization failed; verdict unaffected");
            None
        }
    }
}

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

/// Run a check's failure-evidence capture, then tear its browser
/// context down (#202 / #214 / #215). `wants_capture` is the capture
/// policy's decision for this check: when false, nothing is recorded,
/// but the context is still closed — which discards any video the
/// context recorded (recording is a run-level toggle set up front).
/// Returns the artifacts that landed. Warn-never-fail throughout;
/// evidence gathering never masks or manufactures a verdict.
pub(crate) async fn finalize_capture(
    writer: &mut EvidenceWriter,
    cb: CheckBrowser,
    wants_capture: bool,
    last_step: u32,
    targets: &[TargetLocator],
) -> Vec<CapturedArtifact> {
    let mut captured = Vec::new();
    if wants_capture {
        captured = capture_failure_evidence(writer, &cb.page, last_step).await;
        if let Some(c) = capture_target_rects(writer, &cb.page, last_step, targets).await {
            captured.push(c);
        }
    }
    // Closing the context finalizes any recorded video (#215) and hands
    // its bytes back; keep them only when the policy wanted this check's
    // evidence.
    match cb.close().await {
        Ok(Some(video)) if wants_capture => {
            if let Some(c) = capture_video(writer, last_step, &video).await {
                captured.push(c);
            }
        }
        Ok(_) => {}
        Err(e) => debug!(error = %e, "check context close failed; verdict unaffected"),
    }
    captured
}

/// Record a check's recorded video (#215) as a `capture/video` blob.
/// The bytes arrive from [`duhem_actions::CheckBrowser::close`] —
/// closing the context is what finalizes the recording — so there's no
/// page op and no deadline here, only a size cap. Warn-never-fail: an
/// oversized or unwritable video drops silently, never touching the
/// verdict.
pub(crate) async fn capture_video(
    writer: &mut EvidenceWriter,
    step_index: u32,
    bytes: &[u8],
) -> Option<CapturedArtifact> {
    if bytes.is_empty() {
        return None;
    }
    if bytes.len() > VIDEO_MAX_BYTES {
        warn!(
            bytes = bytes.len(),
            cap = VIDEO_MAX_BYTES,
            "video capture over size cap; skipped (verdict unaffected)"
        );
        return None;
    }
    append_capture(writer, step_index, CAPTURE_VIDEO, bytes).await
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

    async fn writer() -> (EvidenceWriter, tempfile::TempDir) {
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            duhem_evidence::SqliteStore::open(tmp.path().join("d.db"))
                .await
                .unwrap(),
        );
        let w = EvidenceWriter::begin(
            store,
            "01VIDEOCAPTURE0000000000AA",
            "v.yml",
            Default::default(),
        )
        .await
        .unwrap();
        (w, tmp)
    }

    #[tokio::test]
    async fn capture_video_records_a_small_clip() {
        let (mut w, _tmp) = writer().await;
        // WebM EBML magic + a little payload — a plausible tiny clip.
        let mut clip = vec![0x1A, 0x45, 0xDF, 0xA3];
        clip.extend_from_slice(&[0u8; 512]);
        let art = capture_video(&mut w, 0, &clip).await;
        let art = art.expect("a small clip is recorded");
        assert_eq!(art.kind, CAPTURE_VIDEO);
    }

    #[tokio::test]
    async fn capture_video_skips_empty_and_oversized() {
        let (mut w, _tmp) = writer().await;
        // Empty → nothing to record.
        assert!(capture_video(&mut w, 0, &[]).await.is_none());
        // Over the cap → warn + skip, never a blob (evidence never
        // balloons the store, never touches the verdict).
        let huge = vec![0u8; VIDEO_MAX_BYTES + 1];
        assert!(capture_video(&mut w, 0, &huge).await.is_none());
    }
}
