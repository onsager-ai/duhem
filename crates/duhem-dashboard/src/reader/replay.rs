//! Projection of versioned browser-session evidence into the dashboard wire model.

use std::sync::Arc;

use duhem_evidence::{
    SESSION_EVIDENCE_OBSERVATION, SESSION_EVIDENCE_VERSION, SessionEvidence, Store,
};

use super::ReaderError;
use crate::model::{ArtifactRef, ReplayModel, ReplayStep, ReplayVideo};

/// Load the optional replay document referenced by a check's artifacts.
///
/// Missing, malformed, and future-version documents all deliberately degrade
/// to `None`: older runs retain the existing timeline/artifact presentation,
/// while newer contracts are never partially reinterpreted.
pub(super) async fn model(
    store: &Arc<dyn Store>,
    run_id: &str,
    artifacts: &[ArtifactRef],
) -> Result<Option<ReplayModel>, ReaderError> {
    let Some(session_ref) = artifacts
        .iter()
        .find(|artifact| artifact.kind == SESSION_EVIDENCE_OBSERVATION)
    else {
        return Ok(None);
    };
    let Some(bytes) = store.get_blob(&session_ref.id).await? else {
        return Ok(None);
    };
    let Ok(session) = serde_json::from_slice::<SessionEvidence>(&bytes) else {
        return Ok(None);
    };
    if session.version != SESSION_EVIDENCE_VERSION {
        return Ok(None);
    }

    let artifact_for = |sha: &str, kind: &str| {
        artifacts
            .iter()
            .find(|artifact| artifact.id == sha)
            .cloned()
            .unwrap_or_else(|| ArtifactRef {
                id: sha.to_string(),
                kind: kind.to_string(),
                url: format!("/api/runs/{run_id}/artifact/{sha}"),
                mask_counts: Default::default(),
            })
    };
    let steps = session
        .steps
        .into_iter()
        .map(|step| ReplayStep {
            step_index: step.step_index,
            started_ms: step.started_ms,
            finished_ms: step.finished_ms,
            screenshot: step
                .screenshot_sha256
                .as_deref()
                .map(|sha| artifact_for(sha, "capture/step-screenshot")),
            screenshot_ms: step.screenshot_ms,
            frame_error: step.frame_error,
        })
        .collect();
    let video = session.video.map(|video| ReplayVideo {
        started_ms: video.started_ms,
        finished_ms: video.finished_ms,
        artifact: artifact_for(&video.blob_sha256, "capture/video"),
    });

    Ok(Some(ReplayModel {
        version: session.version,
        clock: session.clock,
        duration_ms: session.duration_ms,
        steps,
        network: session.network,
        performance: session.performance,
        video,
    }))
}
