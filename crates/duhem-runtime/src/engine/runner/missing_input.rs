//! Early terminal outcome for a required input that was not supplied.

use duhem_evidence::{EventPayload, EvidenceWriter, VerdictState};
use duhem_judge::{InconclusiveCause, RunVerdict};

use super::{EngineError, RunOutcome};

pub(super) async fn finish(
    writer: &mut EvidenceWriter,
    name: &str,
    run_id: &str,
) -> Result<RunOutcome, EngineError> {
    let verdict = RunVerdict {
        state: VerdictState::Inconclusive(InconclusiveCause::EnvironmentError),
        criteria: Vec::new(),
    };
    writer
        .append(EventPayload::RunFinished {
            verdict: Some(verdict.state),
        })
        .await?;
    Ok(RunOutcome {
        verdict,
        run_id: run_id.to_string(),
        failures: Vec::new(),
        warnings: vec![format!(
            "missing required input `{name}`; supply its declared env variable, a selected profile value, or --inputs"
        )],
    })
}
