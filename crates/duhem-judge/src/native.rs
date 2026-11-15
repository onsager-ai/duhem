//! Native results contribute only to the effective run gate.
use crate::VerdictState;

/// Fold required native outcomes over an independently computed body verdict.
/// A failure dominates inconclusive, which dominates pass, as for checks.
pub fn native_gate(
    body: VerdictState,
    outcomes: impl IntoIterator<Item = (bool, VerdictState)>,
) -> VerdictState {
    let mut gate = body;
    for (required, outcome) in outcomes {
        if !required {
            continue;
        }
        gate = match (gate, outcome) {
            (VerdictState::Fail, _) | (_, VerdictState::Fail) => VerdictState::Fail,
            (VerdictState::Inconclusive(cause), _) | (_, VerdictState::Inconclusive(cause)) => {
                VerdictState::Inconclusive(cause)
            }
            _ => VerdictState::Pass,
        };
    }
    gate
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_and_diagnostic_gate() {
        assert_eq!(
            native_gate(VerdictState::Pass, [(true, VerdictState::Fail)]),
            VerdictState::Fail
        );
        assert_eq!(
            native_gate(VerdictState::Pass, [(false, VerdictState::Fail)]),
            VerdictState::Pass
        );
        let timeout = VerdictState::Inconclusive(crate::InconclusiveCause::Timeout);
        assert_eq!(native_gate(VerdictState::Pass, [(true, timeout)]), timeout);
        assert_eq!(
            native_gate(timeout, [(true, VerdictState::Fail)]),
            VerdictState::Fail
        );
    }
}
