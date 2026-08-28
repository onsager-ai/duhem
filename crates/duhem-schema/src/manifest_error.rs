//! Errors emitted while discovering and loading Verification Definitions.
//!
//! Split out of `manifest.rs` (#472) to keep the loader and its error
//! vocabulary independently within the per-file token budget.

use std::path::PathBuf;

use thiserror::Error;

use crate::verification::SchemaError;

/// Errors from [`crate::load`]. Distinct from [`SchemaError`] because
/// load-time problems span filesystem I/O, path discipline, and the
/// manifest/leaf-shape discriminator — not just YAML parsing.
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: SchemaError,
    },
    /// A file declares both `verifications:` and `criteria:` — load
    /// cannot pick a shape.
    #[error(
        "{path}: cannot be both a root manifest and a Verification Definition (has both `verifications:` and `criteria:`)"
    )]
    AmbiguousShape { path: PathBuf },
    /// A file declares neither `verifications:` nor `criteria:`.
    #[error(
        "{path}: not a Verification Definition or root manifest (missing both `verifications:` and `criteria:`)"
    )]
    UnknownShape { path: PathBuf },
    #[error(
        "{manifest}: entry path `{entry}` is absolute; only paths relative to the manifest are allowed"
    )]
    AbsolutePath { manifest: PathBuf, entry: PathBuf },
    #[error("{manifest}: entry path `{entry}` escapes the manifest's parent directory via `..`")]
    PathEscape { manifest: PathBuf, entry: PathBuf },
    #[error("{manifest}: self-reference cycle on `{entry}`")]
    SelfReference { manifest: PathBuf, entry: PathBuf },
    #[error("{manifest}: glob pattern `{pattern}` is invalid: {source}")]
    InvalidGlob {
        manifest: PathBuf,
        pattern: String,
        #[source]
        source: glob::PatternError,
    },
    /// A `glob:` entry is absolute or contains `..` segments — same
    /// path discipline as `path:` entries.
    #[error(
        "{manifest}: glob pattern `{pattern}` is absolute or escapes the manifest's parent directory via `..`"
    )]
    UnconstrainedGlob { manifest: PathBuf, pattern: String },
    /// A `glob:` match resolved outside the manifest's parent
    /// directory (e.g. via a symlink). Surfaced separately from
    /// `PathEscape` so the diagnostic names the actually-matched file.
    #[error("{manifest}: glob match `{entry}` lies outside the manifest's parent directory")]
    GlobMatchEscaped { manifest: PathBuf, entry: PathBuf },
    /// The manifest declares a `manifest_version` this loader does
    /// not understand. v1 is the only supported value today; future
    /// shape changes bump this and older loaders fail loudly rather
    /// than silently misinterpreting.
    #[error("{path}: unsupported manifest_version {found} (this loader understands {supported})")]
    UnsupportedManifestVersion {
        path: PathBuf,
        found: u32,
        supported: u32,
    },
    #[error("directory `{path}` has no `duhem.yml`")]
    DirectoryMissingManifest { path: PathBuf },
    #[error("directory `{directory}` matches no verifications resolved by manifest `{manifest}`")]
    DirectoryMatchesNoVerifications {
        directory: PathBuf,
        manifest: PathBuf,
    },
    /// Discovery walked the requested directory (or cwd) and its
    /// ancestors (capped at a `.git` repository boundary) without
    /// finding any of the manifest candidate filenames. The `searched`
    /// list names every path probed so the author can see where Duhem
    /// looked.
    #[error(
        "no manifest found in the directory or its ancestors; searched {searched:?}; run `duhem init` to create one"
    )]
    ManifestNotFound { searched: Vec<PathBuf> },
    /// A `profiles:` key is not a well-formed profile name
    /// (lowercase letters, digits, dashes; alphanumeric at both ends).
    #[error(
        "{manifest}: profile name `{name}` is not well-formed (use lowercase letters, digits, and dashes)"
    )]
    MalformedProfileName { manifest: PathBuf, name: String },
    /// A `profiles:` entry declares an empty key map. A named
    /// profile with no keys supplies nothing and is almost
    /// certainly an authoring mistake.
    #[error("{manifest}: profile `{name}` declares no keys")]
    EmptyProfile { manifest: PathBuf, name: String },
    /// An `includes:` chain re-enters a file already on the chain —
    /// resolving it would loop forever. Both ends are named: the file
    /// that declared the offending include and the include target it
    /// points back to.
    #[error("{manifest}: include `{target}` forms a cycle (already in the include chain)")]
    IncludeCycle { manifest: PathBuf, target: PathBuf },
    /// An `includes:` chain nests deeper than the supported maximum.
    /// Guards against pathological fan-out and keeps the merge bounded.
    #[error("{manifest}: include `{target}` exceeds the maximum include depth of {max}")]
    IncludeDepthExceeded {
        manifest: PathBuf,
        target: PathBuf,
        max: usize,
    },
    /// The suite-wide `project:` block is malformed (#191): zero or
    /// multiple coordinate fields, or an empty coordinate.
    #[error("{path}: {message}")]
    BadProjectDecl { path: PathBuf, message: String },
    /// Manifest declarations use the leaf input authoring rules.
    #[error("{path}: manifest input validation failed: {message}")]
    InvalidManifestInputs { path: PathBuf, message: String },
    /// A reusable flow is structurally invalid or cannot be expanded.
    #[error("{path}: flow validation failed: {message}")]
    InvalidFlows { path: PathBuf, message: String },
    /// A leaf loaded by explicit path (issue #384: `-f`/`--file`, or an
    /// explicit positional file) references `$pages.*` and/or a
    /// `call:` flow that its own `pages:` / `flows:` blocks don't
    /// define, and walking up from its directory found no root
    /// manifest to supply them. Distinct from `InvalidFlows` so the
    /// diagnostic names exactly what's unresolved and where it would
    /// normally live, instead of the flow expander's generic "unknown
    /// flow" parse error.
    #[error("{path}: {detail}")]
    UnresolvedWithoutRootManifest { path: PathBuf, detail: String },
}
