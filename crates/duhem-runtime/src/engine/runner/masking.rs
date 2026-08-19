//! Reporter-boundary masking for completed runtime outcomes.

use duhem_evidence::EvidenceWriter;

use super::RunOutcome;

/// The structured reporter is the terminal sink's input. Mask it before
/// returning so pretty/JSON/JUnit reporters and direct programmatic
/// renderers cannot diverge from the evidence boundary.
pub(super) fn mask_run_outcome(writer: &EvidenceWriter, outcome: &mut RunOutcome) {
    for warning in &mut outcome.warnings {
        *warning = writer.mask_text(warning);
    }
    for failure in &mut outcome.failures {
        failure.criterion_id = writer.mask_text(&failure.criterion_id);
        failure.check_id = writer.mask_text(&failure.check_id);
        for assertion in &mut failure.assertions {
            assertion.expr = writer.mask_text(&assertion.expr);
            if let Some(detail) = &mut assertion.detail {
                *detail = writer.mask_text(detail);
            }
        }
    }
    for failure in &mut outcome.cleanup {
        failure.step = writer.mask_text(&failure.step);
        if let Some(detail) = &mut failure.detail {
            *detail = writer.mask_text(detail);
        }
    }
}
