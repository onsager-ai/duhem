//! Verification Definition types and validator.
//!
//! Owns the on-the-wire shape of a Verification Definition — criteria
//! (the human commitment about what "done" means) and checks (the
//! mechanically-judged assertions that verify it). Per
//! `docs/duhem-spec.md` §7.2 / §7.3, criteria are stable; checks are
//! derivative.

pub mod assertion;
pub mod criterion;
pub mod expr;
mod flows;
pub mod includes;
pub mod jsonschema;
mod leaf_context;
pub mod manifest;
mod manifest_compose;
mod manifest_error;
mod manifest_filter;
mod manifest_inputs;
mod manifest_path;
pub mod page_template;
pub mod project;
pub mod provision;
mod source;
pub mod step;
pub mod validate;
mod validate_actions;
pub mod validate_error;
mod validate_lifecycle;
mod validate_pages;
mod validate_runtime;
pub mod verification;
mod viewport;

pub use assertion::{Assertion, TypeCheckKind};
pub use criterion::{Check, Criterion, StepCountError};
pub use expr::{
    BinOp, Expr, ExprStr, Literal, ParseError, Path, PathRoot, RuntimeHelper, RuntimeHelperArity,
    UnaryOp, authored_runtime_helper_names,
};
pub use includes::PartialRootManifest;
pub use jsonschema::json_schema;
pub use manifest::{
    InconclusivePolicy, LoadError, Loaded, LoadedLeaf, ManifestDefaults, ManifestEntry,
    RetryBackoff, RetryPolicy, RootManifest, discover, load, load_for_run,
};
pub use manifest_filter::filter_loaded_to_directory;
pub use project::{ProjectDecl, ProjectKind};
pub use provision::{DurationSpec, HttpReadyProbe, Provision, ReadyProbe};
pub use source::{SourceLocation, SourceMap};
pub use step::{ExpandedFlowOrigin, Step, StepCondition};
pub use validate::{
    ValidationError, validate, validate_with_action_catalog, validate_with_contract_outputs,
};
pub use verification::{
    Fixture, FixtureCatalog, Flow, FlowCatalog, InputDecl, InputType, PageCatalog, SchemaError,
    VerificationDefinition,
};
pub use viewport::Viewport;

/// Current Verification Definition schema version. Pre-1.0 per
/// `docs/duhem-spec.md` §11.3 — breaking changes bump the minor under
/// v0.x, additive changes bump patch, clarifying changes don't bump.
/// The versioning policy lives in the spec issue that introduced this
/// constant; see `CHANGELOG.md` for the rolling ledger.
pub const SCHEMA_VERSION: &str = schema_version!();

/// `concat!`-friendly form of [`SCHEMA_VERSION`]. Exists because
/// `concat!` only accepts literal tokens, not `&'static str` consts —
/// callers that need the schema version baked into a compile-time
/// string (e.g. the CLI's `--version` line) reach for this. Kept in
/// sync with `SCHEMA_VERSION` by `version_macro_matches_const`.
#[macro_export]
macro_rules! schema_version {
    () => {
        "0.3.0"
    };
}

#[cfg(test)]
mod schema_version_tests {
    use super::SCHEMA_VERSION;

    #[test]
    fn parses_as_semver_triple() {
        let parts: Vec<&str> = SCHEMA_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "expected MAJOR.MINOR.PATCH");
        for p in &parts {
            assert!(!p.is_empty(), "empty component in `{SCHEMA_VERSION}`");
            assert!(
                p.chars().all(|c| c.is_ascii_digit()),
                "non-numeric component `{p}` in `{SCHEMA_VERSION}`"
            );
        }
    }

    #[test]
    fn version_macro_matches_const() {
        assert_eq!(SCHEMA_VERSION, schema_version!());
    }
}
