//! Runtime registration of scalar step outputs as secrets (spec #355).
//!
//! A declaration names a path into the raw action result, not an
//! `outputs:` alias. Validation can prove only that the path starts at
//! a contract-declared output; the returned JSON shape is known here.
//! Resolve and check every path before mutating the writer so an invalid
//! object/array declaration leaves no partially registered step state.

use std::collections::BTreeSet;

use duhem_evidence::EvidenceWriter;
use duhem_schema::Step;

use crate::engine::outcome::EngineError;

/// Register the union of authored and action-contract secret paths.
/// The writer source name makes acquired values distinguishable from
/// declared inputs: `[redacted:<step>.<path>]`.
pub(crate) fn register(
    writer: &mut EvidenceWriter,
    step: &Step,
    step_index: usize,
    contract_paths: &[&str],
    outputs: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<(), EngineError> {
    let paths: BTreeSet<&str> = step
        .secret
        .iter()
        .map(String::as_str)
        .chain(contract_paths.iter().copied())
        .collect();
    if paths.is_empty() {
        return Ok(());
    }

    let step_name = step
        .id
        .clone()
        .unwrap_or_else(|| format!("step-{step_index}"));
    let mut resolved = Vec::with_capacity(paths.len());
    for path in paths {
        let value = crate::engine::extract::resolve(outputs, path).ok_or_else(|| {
            EngineError::SecretOutputMissing {
                step: step_name.clone(),
                path: path.to_string(),
            }
        })?;
        let shape = match &value {
            serde_json::Value::Object(_) => Some("object"),
            serde_json::Value::Array(_) => Some("array"),
            _ => None,
        };
        if let Some(shape) = shape {
            return Err(EngineError::SecretOutputNotScalar {
                path: path.to_string(),
                shape,
            });
        }
        resolved.push((format!("{step_name}.{path}"), value));
    }

    for (source, value) in resolved {
        writer.register_secret(source, &value);
    }
    Ok(())
}
