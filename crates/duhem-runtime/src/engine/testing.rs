//! Test-only judgment fixture. Uses the real SQLite query action; never ships
//! in the product catalog. Keeps engine gating/iteration tests independent of
//! browser availability after removal of the old polling actions.
use async_trait::async_trait;
use duhem_actions::{Action, ActionContract, ActionCtx, ActionError, ActionResult, Query};

pub(super) fn contract() -> ActionContract {
    let mut contract = Query.contract();
    contract.uses = "test/sql-judgment";
    contract.outputs.push("satisfied");
    contract
}

struct SqlJudgment;
#[async_trait]
impl Action for SqlJudgment {
    fn uses(&self) -> &'static str {
        "test/sql-judgment"
    }
    fn contract(&self) -> ActionContract {
        contract()
    }
    async fn invoke(
        &self,
        ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let result = Query.invoke(ctx, with).await?;
        let satisfied = result
            .outputs
            .get("row_count")
            .and_then(|value| value.as_u64())
            .is_some_and(|count| count > 0);
        Ok(result.with_output("satisfied", serde_json::json!(satisfied)))
    }
}

pub(super) fn engine() -> super::Engine {
    let mut engine = super::Engine::new();
    engine.register_test_action(Box::new(super::registry::ConcreteAction::new(Box::new(
        SqlJudgment,
    ))));
    engine
}
