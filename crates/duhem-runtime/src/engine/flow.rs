//! Evidence metadata carried by statically expanded flow steps.
//!
//! Dispatch remains the ordinary action path. This helper only moves
//! loader-attached provenance into evidence and registers sensitive
//! parameter bindings before the step-start event crosses the sink.

use duhem_evidence::{EvidenceWriter, FlowOrigin};
use duhem_schema::Step;

use crate::engine::context::RunContext;
use crate::engine::template::substitute_with;

pub(crate) fn register_secrets(writer: &mut EvidenceWriter, step: &Step, context: &RunContext<'_>) {
    for (index, secret) in step.flow_secrets.iter().enumerate() {
        let mut resolved = secret.clone();
        if substitute_with(&mut resolved, context).is_ok()
            && let Ok(value) = serde_json::to_value(&resolved)
        {
            writer.register_secret(
                format!("flow parameter {}:{index}", step.uses_name()),
                &value,
            );
        }
    }
}

pub(crate) fn origin(step: &Step) -> Option<FlowOrigin> {
    step.flow.as_ref().map(|flow| FlowOrigin {
        name: flow.name.clone(),
        invocation: flow.invocation.clone(),
        inner_index: flow.inner_index,
    })
}
