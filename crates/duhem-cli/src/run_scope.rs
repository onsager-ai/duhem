//! Normalized single-leaf versus manifest-run context.
//!
//! `run_cmd` has one execution loop for both shapes. This enum carries
//! only the suite-wide state that changes that loop, keeping manifest
//! additions from expanding the command dispatcher itself.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Evidence namespacing and shared configuration for one invocation.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Scope {
    SingleLeaf,
    Manifest {
        warnings: Vec<String>,
        /// Shared environment provisioned once for the whole suite
        /// (spec #131), anchored at the manifest directory.
        environment: Option<duhem_schema::Environment>,
        manifest_dir: Option<PathBuf>,
        /// Suite-wide engine posture inherited by every leaf (#66).
        defaults: Option<duhem_schema::ManifestDefaults>,
        /// Suite target coordinate; a leaf declaration wins (#191).
        project: Option<duhem_schema::ProjectDecl>,
        /// Declarations consulted by names listed in `inherits:` (#354).
        inputs: BTreeMap<String, duhem_schema::InputDecl>,
    },
}
