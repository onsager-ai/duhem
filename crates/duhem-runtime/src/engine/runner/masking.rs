//! Reporter-boundary masking for completed runtime outcomes.

use duhem_evidence::SecretRegistry;

use super::RunOutcome;

/// The structured reporter is the terminal sink's input. Mask it before
/// returning so pretty/JSON/JUnit reporters and direct programmatic
/// renderers cannot diverge from the evidence boundary.
pub(super) fn mask_run_outcome(secrets: &SecretRegistry, outcome: &mut RunOutcome) {
    for warning in &mut outcome.warnings {
        *warning = secrets.mask(warning).text;
    }
    for failure in &mut outcome.failures {
        failure.criterion_id = secrets.mask(&failure.criterion_id).text;
        failure.check_id = secrets.mask(&failure.check_id).text;
        for assertion in &mut failure.assertions {
            assertion.expr = secrets.mask(&assertion.expr).text;
            if let Some(detail) = &mut assertion.detail {
                *detail = secrets.mask(detail).text;
            }
        }
    }
}
