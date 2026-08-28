//! Small run-detail projections kept out of the evidence reader fold.

use duhem_evidence::RunRecord;

use crate::model::RunSide;

pub(super) fn run_side(r: &RunRecord) -> RunSide {
    RunSide {
        run_id: r.run_id.clone(),
        started_at: Some(r.started_at),
        verdict: r.verdict,
    }
}
