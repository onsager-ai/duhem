use crate::with::TimeoutSpec;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Window {
    pub(crate) from_step: String,
    #[serde(default)]
    pub(crate) edge: Edge,
    #[serde(deserialize_with = "positive_duration")]
    pub(crate) duration: TimeoutSpec,
}

fn positive_duration<'de, D: serde::Deserializer<'de>>(d: D) -> Result<TimeoutSpec, D::Error> {
    let duration = TimeoutSpec::deserialize(d)?;
    if Duration::from(duration).is_zero() {
        return Err(serde::de::Error::custom("window duration must be positive"));
    }
    Ok(duration)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Edge {
    #[default]
    Start,
    Finish,
}

/// Resolve the anchor and wait for its inclusive observation interval.
pub(crate) async fn resolve(
    window: &Window,
    ctx: &crate::ActionCtx<'_>,
    page: &crate::Page,
    action: &str,
) -> Result<(&'static str, f64, f64), crate::ActionError> {
    use crate::ActionError;
    use tokio::time::sleep;
    let boundary = ctx.step_boundaries.get(&window.from_step).ok_or_else(|| {
        ActionError::Playwright(format!(
            "{action}: anchor step `{}` has not run or has no session clock",
            window.from_step
        ))
    })?;
    if boundary.session.as_deref() != ctx.session {
        return Err(ActionError::Playwright(format!(
            "{action}: anchor step `{}` ran in a different browser session",
            window.from_step
        )));
    }
    let duration: Duration = window.duration.into();
    let (edge, start_ms) = match window.edge {
        Edge::Start => ("start", boundary.start_ms),
        Edge::Finish => ("finish", boundary.finish_ms),
    };
    let end_ms = start_ms + duration.as_secs_f64() * 1000.0;
    loop {
        let now = page
            .session_time_ms()
            .await
            .map_err(|e| ActionError::Playwright(format!("{action}: session clock: {e}")))?;
        if now > end_ms {
            break;
        }
        sleep(Duration::from_secs_f64(((end_ms - now) / 1000.0).min(0.05))).await;
    }
    Ok((edge, start_ms, end_ms))
}
