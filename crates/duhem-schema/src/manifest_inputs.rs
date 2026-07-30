//! Manifest-level input declaration discipline (spec #354).
//!
//! A root manifest reuses the leaf [`InputDecl`] shape, so validation
//! must stay shared while diagnostics retain manifest provenance. This
//! module also owns the suite-wide consumption lint: declarations are
//! advisory when temporarily unused, but authors should see stale
//! credential policy instead of silently accumulating it.

use std::collections::BTreeMap;
use std::path::Path;

use crate::manifest::{LoadError, LoadedLeaf};
use crate::verification::InputDecl;

pub(crate) fn validate(path: &Path, inputs: &BTreeMap<String, InputDecl>) -> Result<(), LoadError> {
    let errors = crate::validate::input_decl_errors(inputs, false);
    if errors.is_empty() {
        return Ok(());
    }
    Err(LoadError::InvalidManifestInputs {
        path: path.to_path_buf(),
        message: errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "),
    })
}

pub(crate) fn append_unused_warnings(
    path: &Path,
    inputs: &BTreeMap<String, InputDecl>,
    leaves: &[LoadedLeaf],
    warnings: &mut Vec<String>,
) {
    for name in inputs.keys() {
        if !leaves.iter().any(|leaf| {
            leaf.definition
                .inputs
                .get(name)
                .is_some_and(|decl| decl.inherit)
        }) {
            warnings.push(format!(
                "{}: manifest input `{name}` is not inherited by any verification",
                path.display()
            ));
        }
    }
}
