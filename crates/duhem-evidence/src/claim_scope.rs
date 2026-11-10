//! Reconstruct selection and distinct recorded checks without counting retries.

use std::collections::BTreeSet;

use crate::{Event, EventPayload};

pub fn claim_scope(events: &[Event]) -> Option<duhem_summary::ClaimScope> {
    let mut scope = events.iter().find_map(|event| match &event.payload {
        EventPayload::ClaimScopeDeclared { scope } => Some(scope.clone()),
        _ => None,
    })?;
    let mut checks = BTreeSet::new();
    let mut criteria = BTreeSet::new();
    for event in events {
        if let EventPayload::CheckFinished {
            criterion_id: Some(criterion),
            check_id,
            ..
        } = &event.payload
        {
            criteria.insert(criterion);
            checks.insert((criterion, check_id));
        }
    }
    scope.criteria.executed = criteria.len() as u32;
    scope.checks.executed = checks.len() as u32;
    Some(scope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn retries_and_colliding_check_names_preserve_distinct_claim_identity() {
        let declaration = json!({
            "seq": 0, "ts": "2026-10-30T00:00:00.000Z", "kind": "claim_scope_declared",
            "scope": {"criteria": {"authored": 2, "selected": 2, "executed": 0},
                      "checks": {"authored": 2, "selected": 2, "executed": 0}}
        });
        let mut events = vec![serde_json::from_value(declaration).unwrap()];
        for (index, criterion) in ["A", "A", "B"].iter().enumerate() {
            events.push(
                serde_json::from_value(json!({
                    "seq": index + 1, "ts": "2026-10-30T00:00:01.000Z", "kind": "check_finished",
                    "criterion_id": criterion, "check_id": "same-name", "verdict": "pass"
                }))
                .unwrap(),
            );
        }
        let scope = claim_scope(&events).unwrap();
        assert_eq!(scope.criteria.executed, 2);
        assert_eq!(scope.checks.executed, 2);
        assert!(
            claim_scope(&events[1..]).is_none(),
            "legacy outcomes have unknown authored scope"
        );
    }
}
