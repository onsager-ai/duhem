use super::*;

impl Engine {
    pub(super) fn declared_claim_scope(
        &self,
        definition: &VerificationDefinition,
    ) -> duhem_summary::ClaimScope {
        let mut scope = duhem_summary::ClaimScope {
            filters: self.filter_patterns.clone(),
            manifest: self.manifest_selection.clone(),
            ..Default::default()
        };
        scope.criteria.authored = definition.criteria.len() as u32;
        for criterion in &definition.criteria {
            let mut selected = 0;
            for check in &criterion.checks {
                scope.checks.authored += 1;
                if self
                    .filter
                    .as_ref()
                    .is_none_or(|filter| filter.matches(&criterion.id, &check.id))
                {
                    selected += 1;
                } else {
                    scope.excluded.push(duhem_summary::ExcludedClaim {
                        criterion_id: criterion.id.clone(),
                        check_id: check.id.clone(),
                        reason: "check_filter".into(),
                    });
                }
            }
            scope.checks.selected += selected;
            scope.criteria.selected += u32::from(selected > 0);
        }
        scope
    }
}
