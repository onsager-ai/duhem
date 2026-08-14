//! `RootManifest` — `duhem.yml` at the root of a Verification
//! Definition tree.
//!
//! Lists or globs child Verification Definitions (Patterns B and C
//! from `docs/duhem-spec.md` §10.4). A manifest is *not* itself a
//! Verification Definition — no `criteria:`, no `setup:`. The loader
//! distinguishes manifest from leaf by which key is present.
//!
//! Spec on issue #49.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::manifest_path::{
    canonical_or_self, is_under, validate_entry_path, validate_glob_pattern,
};
use crate::project::ProjectDecl;
use crate::provision::{DurationSpec, Provision};
use crate::verification::{
    FlowCatalog, InputDecl, PageCatalog, SchemaError, VerificationDefinition,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RootManifest {
    /// Document version. v1 today; future schema changes bump this so
    /// a manifest written against today's loader refuses to silently
    /// reinterpret a future shape.
    pub manifest_version: u32,
    /// Other manifest files whose config this one composes in
    /// (`docs/duhem-spec.md` §10.4, spec #67). Each path is relative
    /// to *this* manifest's parent directory and resolves to a
    /// **partial** manifest (every field optional, no
    /// `manifest_version:`) — see [`PartialRootManifest`]. Same path
    /// discipline as `verifications:` entries: no absolute paths, no
    /// `..` escape.
    ///
    /// Composition is **root-wins**: an include fills only the keys
    /// this manifest leaves absent (for nested maps like
    /// `profiles:`, key-by-key, still root-wins per leaf key).
    /// Manifest `inputs:` declarations overlay by input name.
    /// `verifications:` (and `includes:` themselves) are *concatenated*
    /// rather than overlaid — the root's entries first, then each
    /// include's, in declared order. Nested includes are resolved
    /// depth-first up to depth 3; cycles are a hard error. The typical
    /// shape is a team-shared `.duhem.shared.yml` plus a gitignored
    /// per-developer `.duhem.local.yml`. Additive: a manifest without
    /// `includes:` behaves byte-identically to before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<PathBuf>,
    /// Optional provisioning lifecycle shared by the whole suite. When present, the
    /// runtime provisions it **once** (`up:` + `ready:`) before any leaf
    /// runs and tears it down **once** after the last leaf — instead of
    /// each leaf standing up its own. Leaves keep their own
    /// `provision:` so they stay runnable standalone; a manifest run
    /// suppresses per-leaf provisioning and points every leaf at this
    /// shared stack. Additive: a manifest without it behaves as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provision: Option<Provision>,
    /// Named configuration profiles (`docs/duhem-spec.md` §10.4, spec
    /// #68). Each key under `profiles:` is a profile name
    /// (e.g. `staging`, `prod`) whose value is a free-form
    /// `key: value` map. A key inherited through a manifest `inputs:`
    /// declaration is type-checked; all other keys remain conventionally
    /// typed for backward compatibility.
    ///
    /// When a profile is selected for a run (CLI `--profile`,
    /// or auto-selected when exactly one is declared) its keys feed
    /// the leaf input-resolution chain (a profile key `base_url`
    /// populates a declared input `base_url` when no higher-precedence
    /// source supplies it) and its string-valued keys are whitelisted
    /// for `$env.<key>`. Additive: a manifest without `profiles:`
    /// behaves byte-identically to before.
    ///
    /// `serde_yml::Value` has no `JsonSchema` impl, so the field is
    /// described to schemars via a `serde_json::Value`-shaped stand-in
    /// (same wire surface) purely so the JSON-schema artifact
    /// generates.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(
        with = "std::collections::BTreeMap<String, std::collections::BTreeMap<String, serde_json::Value>>"
    )]
    pub profiles: BTreeMap<String, BTreeMap<String, serde_yml::Value>>,
    /// Suite-wide input declarations (spec #354). The manifest owns
    /// type, process-`env:` fallback, and sensitivity; selected
    /// `profiles:` entries continue to own values. A leaf opts into
    /// a declaration with `inputs.<name>.inherit: true`, preserving
    /// explicit dependency injection and a closed leaf declaration set.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, InputDecl>,
    /// Named locator catalog shared by every leaf in the suite.
    /// Entries merge by page and element name under the root-wins rule;
    /// a leaf-local entry overlays the composed manifest value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(
        with = "std::collections::BTreeMap<String, std::collections::BTreeMap<String, serde_json::Value>>"
    )]
    pub pages: PageCatalog,
    /// Suite-wide reusable step flows. Entries merge by name under
    /// root-wins; a leaf-local flow with the same name wins.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flows: FlowCatalog,
    /// Suite-wide defaults (`docs/duhem-spec.md` §10.4, spec #66).
    /// A single `defaults:` block that every leaf the manifest expands
    /// to inherits — so an author sets the timeout budget, the
    /// inconclusive-handling policy, and the retry posture once for the
    /// whole suite instead of per leaf.
    ///
    /// Each sub-key is optional and, when absent, reproduces today's
    /// behavior exactly:
    ///
    /// - `timeout:` is the per-step `timeout:` fallback. A step that
    ///   declares its own `timeout:` wins; a step that doesn't picks up
    ///   this default; with neither, the engine's built-in 5s last
    ///   resort applies.
    /// - `inconclusive_policy:` decides how a criterion-level
    ///   `inconclusive` is treated at run aggregation — `block`
    ///   (today's behavior; inconclusive ≠ pass), `warn` (criterion
    ///   passes but a warning is surfaced in the run summary), or
    ///   `pass` (silent pass). Per-assertion evaluation is unchanged.
    /// - `retry:` re-runs a whole check from step 0 when it comes back
    ///   `inconclusive` for a *retry-eligible* cause (timeout or an
    ///   environment error); a `fail` never retries.
    /// - `profile:` names a key under the sibling `profiles:`
    ///   block. Cross-key validation is deferred to engine-time lookup
    ///   (spec #66 Out-of-scope), so any string is accepted here.
    ///
    /// Additive: a manifest without `defaults:` round-trips and behaves
    /// byte-for-byte as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<ManifestDefaults>,
    /// Optional suite-wide declared target coordinate (#191): what
    /// the whole suite verifies. A leaf's own `project:` wins over
    /// this. Absent → the runtime's identity-resolution ladder falls
    /// through to CI context / normalized remote / path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectDecl>,
    /// Entries that resolve to leaf Verification Definitions. Order
    /// determines execution order across leaves.
    pub verifications: Vec<ManifestEntry>,
}

/// Suite-wide defaults declared once on the root manifest (spec #66).
/// Every sub-key is optional; an absent sub-key falls back to today's
/// behavior (`timeout` → the engine's 5s default, `inconclusive_policy`
/// → `block`, `retry.max` → `0`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestDefaults {
    /// Name of a profile under the sibling `profiles:` block
    /// to use for the suite. Accepted as any string here; the lookup
    /// against `profiles:` happens at engine-time (spec #66
    /// Out-of-scope), so an unknown name is not a parse error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Per-step `timeout:` fallback for every leaf. A step's own
    /// `timeout:` always wins; this only fills in when a step omits it.
    /// Same duration wire shape as `provision.ready.http.timeout`
    /// (integer milliseconds or a suffixed string like `30s` / `2m`).
    /// Absent → the engine's built-in 5s default still applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<DurationSpec>,
    /// How a criterion-level `inconclusive` verdict is treated at run
    /// aggregation. Absent → `block` (today's behavior).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inconclusive_policy: Option<InconclusivePolicy>,
    /// Check-level retry posture. Absent → no retries (`max` is `0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
}

/// How a criterion-level `inconclusive` verdict is treated at run
/// aggregation (spec #66). Closed enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InconclusivePolicy {
    /// Today's behavior: `inconclusive` stays `inconclusive` and does
    /// not pass.
    Block,
    /// Treat a criterion-level `inconclusive` as a pass, but surface a
    /// warning in the run summary so it's not silent.
    Warn,
    /// Silently treat a criterion-level `inconclusive` as a pass.
    Pass,
}

/// Check-level retry posture (spec #66). A whole check re-runs from
/// step 0 when it returns `inconclusive` for a retry-eligible cause
/// (timeout or environment error), matching #54's action-level retry
/// classification. A `fail` never retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    /// Maximum number of retries after the first attempt. `0` (the
    /// default when omitted) means no retries — today's behavior.
    #[serde(default)]
    pub max: u32,
    /// Backoff schedule between retries. Defaults to `exponential`.
    #[serde(default)]
    pub backoff: RetryBackoff,
}

/// Backoff schedule between check retries (spec #66). Closed enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetryBackoff {
    /// Delay doubles each retry (`base`, `2·base`, `4·base`, …).
    #[default]
    Exponential,
    /// Delay grows linearly each retry (`base`, `2·base`, `3·base`, …).
    Linear,
}

/// A well-formed profile name is lowercase ASCII letters, digits,
/// and dashes, beginning and ending with an alphanumeric. This keeps
/// names addressable on the CLI (`--profile <name>`) and stable
/// across the `$env` whitelist surface.
pub(crate) fn profile_name_well_formed(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let bytes = name.as_bytes();
    let edge_ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !edge_ok(bytes[0]) || !edge_ok(bytes[bytes.len() - 1]) {
        return false;
    }
    name.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// One entry in `verifications:`. An author writes either `path:` for
/// an explicit leaf (Pattern B) or `glob:` for a globbed expansion
/// (Pattern C). The two are mutually exclusive at the YAML level
/// (untagged enum), which keeps the wire shape unambiguous.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum ManifestEntry {
    Path {
        /// Path relative to the manifest's parent directory. Absolute
        /// paths and `..` segments are rejected at load time.
        path: PathBuf,
    },
    Glob {
        /// Shell-style glob (`**`, `*`, `?`) expanded against the
        /// manifest's parent directory.
        glob: String,
    },
}

impl RootManifest {
    /// Parse a root manifest from YAML source. Does not resolve
    /// entries; call [`crate::load`] for the full discovery pipeline.
    pub fn from_yaml_str(src: &str) -> Result<Self, SchemaError> {
        serde_yml::from_str(src).map_err(SchemaError::from)
    }
}

/// Outcome of [`crate::load`] — either a single Verification
/// Definition (Pattern A; today's behavior) or a manifest that
/// expanded into N leaves (Patterns B / C).
#[derive(Debug)]
pub enum Loaded {
    Leaf {
        /// Absolute or as-supplied path of the leaf file.
        path: PathBuf,
        definition: VerificationDefinition,
    },
    Manifest {
        /// Path of the manifest file itself.
        manifest_path: PathBuf,
        manifest: RootManifest,
        /// Per-leaf `(path, definition)` pairs in the order they
        /// resolved from the manifest. Globs are pre-expanded and
        /// flattened.
        leaves: Vec<LoadedLeaf>,
        /// Non-fatal load-time warnings (e.g. a glob that matched
        /// nothing). CLI surfaces these to stderr.
        warnings: Vec<String>,
    },
}

#[derive(Debug)]
pub struct LoadedLeaf {
    /// Path on disk of the leaf file.
    pub path: PathBuf,
    /// Parsed Verification Definition.
    pub definition: VerificationDefinition,
}

/// Errors from [`crate::load`]. Distinct from [`SchemaError`] because
/// load-time problems span filesystem I/O, path discipline, and the
/// manifest/leaf-shape discriminator — not just YAML parsing.
#[derive(Debug, thiserror::Error)]
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
    /// Discovery (issue #69) walked the current directory and its
    /// ancestors (capped at a `.git` repository boundary) without
    /// finding any of the manifest candidate filenames. The `searched`
    /// list names every path probed so the author can see where Duhem
    /// looked.
    #[error("no manifest found in the current directory or its ancestors; searched {searched:?}")]
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
    /// An `includes:` chain nests deeper than the supported maximum
    /// ([`MAX_INCLUDE_DEPTH`]). Guards against pathological fan-out and
    /// keeps the merge bounded.
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

/// Currently-supported `manifest_version` value. Bumping this is
/// schema-impacting and requires a `CHANGELOG.md` entry.
pub const SUPPORTED_MANIFEST_VERSION: u32 = 1;

/// Manifest filenames probed when resolving a directory or discovering
/// from the cwd (issue #69), in priority order. The plain `duhem.yml`
/// is the canonical name; `duhem.yaml` is the long-extension variant;
/// the leading-dot `.duhem.*` aliases let a manifest hide like
/// `.gitignore` / `.editorconfig`. Earlier entries win when several
/// exist in the same directory.
pub(crate) const MANIFEST_CANDIDATES: [&str; 4] =
    ["duhem.yml", "duhem.yaml", ".duhem.yml", ".duhem.yaml"];

/// The first existing manifest candidate directly under `dir`, in
/// [`MANIFEST_CANDIDATES`] priority order, or `None` when the
/// directory holds none of them.
pub(crate) fn manifest_in_dir(dir: &Path) -> Option<PathBuf> {
    MANIFEST_CANDIDATES
        .iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Resolve the manifest a `duhem run [path]` invocation targets,
/// without the `-f`/`--file` override (which bypasses discovery and is
/// the caller's concern). `explicit` is the positional `path` argument
/// (`None` when omitted); `cwd` is the directory the walk starts from.
///
/// Resolution order (issue #69 §Design):
///
/// 1. An explicit *file* is used verbatim — Pattern A (a single leaf
///    passed directly) is preserved byte-for-byte.
/// 2. An explicit *directory* is probed for a manifest candidate; a
///    directory with none keeps today's `DirectoryMissingManifest`
///    error. Explicit args behave identically to before this spec —
///    the ancestor walk only activates when no path is given.
/// 3. An explicit path that is neither file nor directory (e.g. a
///    path that does not exist) is handed back verbatim so [`load`]
///    surfaces the same I/O error against the offending path.
/// 4. With no path: probe the cwd, then each ancestor, in
///    [`MANIFEST_CANDIDATES`] priority order. First hit wins. The walk
///    is capped at any `.git` directory (a repository boundary) so a
///    sibling repo's manifest is never picked up by accident.
/// 5. Exhausting the walk yields [`LoadError::ManifestNotFound`]
///    listing every path probed.
pub fn discover(explicit: Option<&Path>, cwd: &Path) -> Result<PathBuf, LoadError> {
    if let Some(path) = explicit {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        if path.is_dir() {
            return manifest_in_dir(path).ok_or_else(|| LoadError::DirectoryMissingManifest {
                path: path.to_path_buf(),
            });
        }
        // Neither file nor directory: defer to `load` for the I/O
        // error, preserving today's diagnostic on a bad explicit path.
        return Ok(path.to_path_buf());
    }

    crate::manifest_compose::walk_ancestors_for_manifest(cwd)
}

/// Resolve a CLI `path` argument to a [`Loaded`].
///
/// Dispatch rules (issue #49 § "CLI surface"):
///
/// - Directory → look for the first [`MANIFEST_CANDIDATES`] entry it
///   contains (`duhem.yml`, `duhem.yaml`, `.duhem.yml`, `.duhem.yaml`).
/// - File with `verifications:` → root manifest; expand entries.
/// - File with `criteria:` → single leaf (today's behavior).
/// - File with both / neither → load-time error.
///
/// On a manifest, every resolved leaf is parsed eagerly; a malformed
/// leaf fails the whole load with a path-tagged error so authors see
/// the offending file.
pub fn load(path: &Path) -> Result<Loaded, LoadError> {
    let (path, src, shape) = resolve_and_classify(path)?;
    match shape {
        Shape::Manifest => load_manifest(&path, &src),
        Shape::Leaf => load_leaf(&path, &src).map(|definition| Loaded::Leaf { path, definition }),
        Shape::Ambiguous => Err(LoadError::AmbiguousShape { path }),
        Shape::Unknown => Err(LoadError::UnknownShape { path }),
    }
}

/// Like [`load`], but a leaf's `$pages.*` and flow references resolve
/// through the nearest root manifest found walking up from its
/// directory (issue #384: [`resolve_leaf_through_root_manifest`]),
/// instead of failing outright when they live in files a manifest's
/// `includes:` composes in. Sibling `verifications:` entries the
/// found manifest lists are never loaded or executed — the run stays
/// scoped to the requested leaf.
///
/// The manifest branch is byte-identical to [`load`] (a target that
/// resolves to a manifest already sees every leaf it lists, so there
/// is nothing to widen); only the leaf branch differs. `duhem run`
/// uses this for its `-f`/`--file` and explicit-path targets; the
/// no-path ancestor walk only ever resolves to a manifest, so it is
/// unaffected. Other callers (`duhem validate`, `duhem resolve`) keep
/// today's self-contained-leaf behavior via [`load`].
pub fn load_for_run(path: &Path) -> Result<Loaded, LoadError> {
    let (path, src, shape) = resolve_and_classify(path)?;
    match shape {
        Shape::Manifest => load_manifest(&path, &src),
        Shape::Leaf => {
            let mut definition = load_leaf_authored(&path, &src)?;
            crate::leaf_context::resolve_leaf_through_root_manifest(&path, &mut definition)?;
            Ok(Loaded::Leaf { path, definition })
        }
        Shape::Ambiguous => Err(LoadError::AmbiguousShape { path }),
        Shape::Unknown => Err(LoadError::UnknownShape { path }),
    }
}

/// Resolve a directory to its manifest candidate (`load`'s `path.is_dir()`
/// probe), read the target file, and classify its shape. Shared by
/// [`load`] and [`load_for_run`] so they never drift on how a target
/// is located — only how a `Shape::Leaf` result is then loaded.
fn resolve_and_classify(path: &Path) -> Result<(PathBuf, String, Shape), LoadError> {
    let path = if path.is_dir() {
        match manifest_in_dir(path) {
            Some(candidate) => candidate,
            None => {
                return Err(LoadError::DirectoryMissingManifest {
                    path: path.to_path_buf(),
                });
            }
        }
    } else {
        path.to_path_buf()
    };

    let src = std::fs::read_to_string(&path).map_err(|e| LoadError::Io {
        path: path.clone(),
        source: e,
    })?;

    let shape = classify_yaml(&path, &src)?;
    Ok((path, src, shape))
}

enum Shape {
    Manifest,
    Leaf,
    Ambiguous,
    Unknown,
}

/// Top-level key sniff. We parse the YAML once as an untyped Mapping
/// and check which discriminator key is present. Both `verifications:`
/// and `criteria:` → ambiguous; neither → unknown. A YAML parse
/// failure surfaces as `LoadError::Yaml` so the user sees the real
/// line/column rather than a confusing "unknown shape" message.
fn classify_yaml(path: &Path, src: &str) -> Result<Shape, LoadError> {
    let value: serde_yml::Value = serde_yml::from_str(src).map_err(|e| LoadError::Yaml {
        path: path.to_path_buf(),
        source: SchemaError::from(e),
    })?;
    let map = match value.as_mapping() {
        Some(m) => m,
        // Non-mapping documents (e.g. `null`, a top-level sequence)
        // are real authoring mistakes, not shape ambiguity — surface
        // as "unknown shape" so the diagnostic names both
        // discriminator keys.
        None => return Ok(Shape::Unknown),
    };
    let has_verifications = map.contains_key(serde_yml::Value::String("verifications".into()));
    let has_criteria = map.contains_key(serde_yml::Value::String("criteria".into()));
    Ok(match (has_verifications, has_criteria) {
        (true, true) => Shape::Ambiguous,
        (true, false) => Shape::Manifest,
        (false, true) => Shape::Leaf,
        (false, false) => Shape::Unknown,
    })
}

fn load_leaf_authored(path: &Path, src: &str) -> Result<VerificationDefinition, LoadError> {
    VerificationDefinition::from_yaml_str(src).map_err(|e| LoadError::Yaml {
        path: path.to_path_buf(),
        source: e,
    })
}

fn load_leaf(path: &Path, src: &str) -> Result<VerificationDefinition, LoadError> {
    let mut definition = load_leaf_authored(path, src)?;
    crate::flows::validate_and_expand(&mut definition).map_err(|message| {
        LoadError::InvalidFlows {
            path: path.to_path_buf(),
            message,
        }
    })?;
    Ok(definition)
}

fn load_manifest(manifest_path: &Path, src: &str) -> Result<Loaded, LoadError> {
    let manifest = crate::manifest_compose::compose_manifest(manifest_path, src)?;
    let parent = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let manifest_canonical = canonical_or_self(manifest_path);
    // Pre-canonicalize the manifest's parent so we can verify that
    // every glob hit stays inside it. `canonical_or_self` falls back
    // to the input on `canonicalize` failure (e.g. relative paths in
    // tests), which still gives us a stable prefix to compare against.
    let parent_canonical = canonical_or_self(parent);

    let mut leaves: Vec<LoadedLeaf> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();

    for entry in &manifest.verifications {
        let resolved_paths = match entry {
            ManifestEntry::Path { path } => {
                validate_entry_path(manifest_path, path)?;
                vec![parent.join(path)]
            }
            ManifestEntry::Glob { glob: pattern } => {
                // Same path discipline as `path:` entries — `glob:`
                // is not an escape hatch out of the manifest tree.
                validate_glob_pattern(manifest_path, pattern)?;
                let joined = parent.join(pattern);
                let pattern_str = joined.to_string_lossy().into_owned();
                let matches = glob::glob(&pattern_str).map_err(|e| LoadError::InvalidGlob {
                    manifest: manifest_path.to_path_buf(),
                    pattern: pattern.clone(),
                    source: e,
                })?;
                let mut hits: Vec<PathBuf> = Vec::new();
                for p in matches.flatten() {
                    // Symlinks / weird filesystem shapes can land a
                    // match outside the manifest's parent tree even
                    // when the pattern was well-behaved. Reject those
                    // explicitly so the spec's "Patterns are
                    // normalized: no `..` escaping the manifest's
                    // parent dir" guarantee survives the expansion.
                    if !is_under(&parent_canonical, &p) {
                        return Err(LoadError::GlobMatchEscaped {
                            manifest: manifest_path.to_path_buf(),
                            entry: p,
                        });
                    }
                    hits.push(p);
                }
                // Self-only globs (the glob expanded to *just* the
                // manifest itself) are a real cycle error: surface
                // them rather than silently warning.
                let only_self = !hits.is_empty()
                    && hits
                        .iter()
                        .all(|p| canonical_or_self(p) == manifest_canonical);
                if only_self {
                    return Err(LoadError::SelfReference {
                        manifest: manifest_path.to_path_buf(),
                        entry: PathBuf::from(pattern.clone()),
                    });
                }
                if hits.is_empty() {
                    warnings.push(format!(
                        "{}: glob `{}` matched no files",
                        manifest_path.display(),
                        pattern
                    ));
                }
                hits
            }
        };

        for leaf_path in resolved_paths {
            // Globs may surface the manifest itself; explicit
            // `path:` entries get the same self-reference check.
            if canonical_or_self(&leaf_path) == manifest_canonical {
                return Err(LoadError::SelfReference {
                    manifest: manifest_path.to_path_buf(),
                    entry: leaf_path.clone(),
                });
            }
            // Dedup repeated paths (e.g. two overlapping globs).
            // First occurrence wins, keeping authored order.
            let canon = canonical_or_self(&leaf_path);
            if !seen.insert(canon.clone()) {
                continue;
            }
            let src = std::fs::read_to_string(&leaf_path).map_err(|e| LoadError::Io {
                path: leaf_path.clone(),
                source: e,
            })?;
            // Each resolved leaf must be a Verification Definition.
            // A nested manifest is a real authoring mistake, not a
            // composition feature in v1.
            match classify_yaml(&leaf_path, &src)? {
                Shape::Leaf => {}
                Shape::Manifest => {
                    return Err(LoadError::UnknownShape {
                        path: leaf_path.clone(),
                    });
                }
                Shape::Ambiguous => {
                    return Err(LoadError::AmbiguousShape {
                        path: leaf_path.clone(),
                    });
                }
                Shape::Unknown => {
                    return Err(LoadError::UnknownShape {
                        path: leaf_path.clone(),
                    });
                }
            }
            let mut def = load_leaf_authored(&leaf_path, &src)?;
            // Leaf-local entries win over the effective manifest
            // catalog, element by element.
            crate::manifest_compose::merge_manifest_catalogs_into_leaf(&manifest, &mut def);
            crate::flows::validate_and_expand(&mut def).map_err(|message| {
                LoadError::InvalidFlows {
                    path: leaf_path.clone(),
                    message,
                }
            })?;
            leaves.push(LoadedLeaf {
                path: leaf_path,
                definition: def,
            });
        }
    }

    crate::manifest_inputs::append_unused_warnings(
        manifest_path,
        &manifest.inputs,
        &leaves,
        &mut warnings,
    );

    Ok(Loaded::Manifest {
        manifest_path: manifest_path.to_path_buf(),
        manifest,
        leaves,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }

    const LEAF_A: &str = r#"
verification: leaf-a
criteria:
  - id: AC-1
    description: trivial
    checks:
      - id: AC-1.1
        assertions:
          - "true"
"#;

    const LEAF_B: &str = r#"
verification: leaf-b
criteria:
  - id: AC-1
    description: trivial
    checks:
      - id: AC-1.1
        assertions:
          - "true"
"#;

    #[test]
    fn parses_minimal_manifest() {
        let y = r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
  - glob: ./**/duhem.yml
"#;
        let m = RootManifest::from_yaml_str(y).expect("parse");
        assert_eq!(m.manifest_version, 1);
        assert_eq!(m.verifications.len(), 2);
        match &m.verifications[0] {
            ManifestEntry::Path { path } => assert_eq!(path, &PathBuf::from("./a/duhem.yml")),
            _ => panic!("expected path entry"),
        }
        match &m.verifications[1] {
            ManifestEntry::Glob { glob } => assert_eq!(glob, "./**/duhem.yml"),
            _ => panic!("expected glob entry"),
        }
    }

    #[test]
    fn pages_round_trip_and_absence_keeps_manifest_wire_shape() {
        let absent =
            RootManifest::from_yaml_str("manifest_version: 1\nverifications: []\n").unwrap();
        assert!(absent.pages.is_empty());
        assert!(!serde_yml::to_string(&absent).unwrap().contains("pages:"));

        let parsed = RootManifest::from_yaml_str(
            "manifest_version: 1\npages:\n  login:\n    submit: { role: button, name: Sign In }\nverifications: []\n",
        )
        .expect("parse pages");
        let round_trip =
            RootManifest::from_yaml_str(&serde_yml::to_string(&parsed).unwrap()).unwrap();
        assert_eq!(parsed, round_trip);
    }

    #[test]
    fn flows_round_trip_and_absence_keeps_manifest_wire_shape() {
        let absent =
            RootManifest::from_yaml_str("manifest_version: 1\nverifications: []\n").unwrap();
        assert!(absent.flows.is_empty());
        assert!(!serde_yml::to_string(&absent).unwrap().contains("flows:"));

        let parsed = RootManifest::from_yaml_str(
            "manifest_version: 1\nflows:\n  shared:\n    steps:\n      - uses: cli/invoke\nverifications: []\n",
        )
        .expect("parse flows");
        let round_trip =
            RootManifest::from_yaml_str(&serde_yml::to_string(&parsed).unwrap()).unwrap();
        assert_eq!(parsed, round_trip);
    }

    #[test]
    fn manifest_rejects_unknown_top_level_field() {
        let y = "manifest_version: 1\nverifications: []\nfoo: bar\n";
        assert!(RootManifest::from_yaml_str(y).is_err());
    }

    #[test]
    fn teardown_is_leaf_scoped_and_rejected_on_a_manifest() {
        let y = r#"
manifest_version: 1
teardown:
  - uses: cli/invoke
    with: { command: [sh, -c, "true"] }
verifications: []
"#;
        let error = RootManifest::from_yaml_str(y).unwrap_err();
        assert!(error.to_string().contains("unknown field `teardown`"));
    }

    #[test]
    fn load_pattern_b_resolves_explicit_paths() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(tmp.path(), "b/duhem.yml", LEAF_B);
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
  - path: ./b/duhem.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest {
                leaves, warnings, ..
            } => {
                assert_eq!(leaves.len(), 2);
                assert_eq!(leaves[0].definition.verification, "leaf-a");
                assert_eq!(leaves[1].definition.verification, "leaf-b");
                assert!(warnings.is_empty());
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn load_pattern_c_glob_resolves_leaves() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(tmp.path(), "b/duhem.yml", LEAF_B);
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - glob: ./*/duhem.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest {
                leaves, warnings, ..
            } => {
                let names: Vec<&str> = leaves
                    .iter()
                    .map(|l| l.definition.verification.as_str())
                    .collect();
                assert_eq!(names.len(), 2);
                assert!(names.contains(&"leaf-a"));
                assert!(names.contains(&"leaf-b"));
                assert!(warnings.is_empty());
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn zero_match_glob_warns_but_loads() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - glob: ./nope/*.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest {
                leaves, warnings, ..
            } => {
                assert!(leaves.is_empty());
                assert_eq!(warnings.len(), 1);
                assert!(warnings[0].contains("matched no files"), "{warnings:?}");
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn self_referential_path_is_cycle_error() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./duhem.yml
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::SelfReference { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn self_only_glob_is_cycle_error() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - glob: ./duhem.yml
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::SelfReference { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn absolute_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let abs = tmp.path().join("a").join("duhem.yml");
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(
            tmp.path(),
            "duhem.yml",
            &format!(
                r#"
manifest_version: 1
verifications:
  - path: {}
"#,
                abs.display()
            ),
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(matches!(err, LoadError::AbsolutePath { .. }), "got {err:?}");
    }

    #[test]
    fn parent_dir_escape_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ../duhem.yml
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(matches!(err, LoadError::PathEscape { .. }), "got {err:?}");
    }

    #[test]
    fn ambiguous_shape_is_load_error() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
verifications:
  - path: ./a/duhem.yml
criteria: []
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::AmbiguousShape { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn unknown_shape_is_load_error() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "duhem.yml", "verification: x\n");
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(matches!(err, LoadError::UnknownShape { .. }), "got {err:?}");
    }

    #[test]
    fn load_leaf_returns_leaf_variant() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(tmp.path(), "leaf.yml", LEAF_A);
        match load(&p).unwrap() {
            Loaded::Leaf { definition, .. } => assert_eq!(definition.verification, "leaf-a"),
            _ => panic!("expected Leaf"),
        }
    }

    #[test]
    fn directory_resolves_to_duhem_yml() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
"#,
        );
        let loaded = load(tmp.path()).unwrap();
        match loaded {
            Loaded::Manifest { leaves, .. } => assert_eq!(leaves.len(), 1),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn directory_without_manifest_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let err = load(tmp.path()).unwrap_err();
        assert!(
            matches!(err, LoadError::DirectoryMissingManifest { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn malformed_leaf_surfaces_offending_path() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", "criteria: not-a-sequence\n");
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        match err {
            LoadError::Yaml { path, .. } => {
                assert!(
                    path.to_string_lossy().contains("a/duhem.yml"),
                    "got {path:?}"
                );
            }
            other => panic!("expected Yaml, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_manifest_version_is_load_error() {
        // Older loaders must refuse a future shape rather than
        // silently misinterpreting it.
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 2
verifications: []
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        match err {
            LoadError::UnsupportedManifestVersion {
                found, supported, ..
            } => {
                assert_eq!(found, 2);
                assert_eq!(supported, SUPPORTED_MANIFEST_VERSION);
            }
            other => panic!("expected UnsupportedManifestVersion, got {other:?}"),
        }
    }

    #[test]
    fn malformed_yaml_surfaces_parse_error_not_unknown_shape() {
        // YAML parse failure should produce `LoadError::Yaml` with
        // line/column context, not a "missing both discriminator
        // keys" message.
        let tmp = tempfile::tempdir().unwrap();
        // Tab where YAML expects spaces — produces a real Yaml
        // location error.
        write(tmp.path(), "duhem.yml", "criteria:\n\t- id: AC-1\n");
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::Yaml { .. }),
            "expected Yaml parse error, got {err:?}"
        );
    }

    #[test]
    fn glob_with_parent_dir_segment_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - glob: ../**/duhem.yml
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::UnconstrainedGlob { .. }),
            "expected UnconstrainedGlob, got {err:?}"
        );
    }

    #[test]
    fn absolute_glob_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - glob: /tmp/**/duhem.yml
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::UnconstrainedGlob { .. }),
            "expected UnconstrainedGlob, got {err:?}"
        );
    }

    #[test]
    fn profiles_round_trip_preserves_nested_maps() {
        let y = r#"
manifest_version: 1
profiles:
  staging:
    base_url: https://staging.example.com
    db_url: postgres://staging-db
    workers: 3
  prod:
    base_url: https://example.com
verifications: []
"#;
        let m = RootManifest::from_yaml_str(y).expect("parse");
        assert_eq!(m.profiles.len(), 2);
        let staging = &m.profiles["staging"];
        assert_eq!(
            staging["base_url"],
            serde_yml::Value::String("https://staging.example.com".into())
        );
        assert_eq!(staging["db_url"].as_str(), Some("postgres://staging-db"));
        assert_eq!(staging["workers"].as_u64(), Some(3));
        assert_eq!(
            m.profiles["prod"]["base_url"].as_str(),
            Some("https://example.com")
        );
        // Old schema spellings are not transitional aliases.
        for old in [
            "environments: {}\n",
            "environment:\n  up: ./scripts/up.sh\n",
            "defaults:\n  environment: staging\n",
        ] {
            let bad = format!("manifest_version: 1\n{old}verifications: []\n");
            let err = RootManifest::from_yaml_str(&bad).unwrap_err();
            assert!(format!("{err}").contains("unknown field"), "got: {err}");
        }
    }

    #[test]
    fn absent_profiles_default_to_empty() {
        let y = "manifest_version: 1\nverifications: []\n";
        let m = RootManifest::from_yaml_str(y).expect("parse");
        assert!(m.profiles.is_empty());
        // Round-trips back out without an `profiles:` key.
        let out = serde_yml::to_string(&m).unwrap();
        assert!(!out.contains("profiles"), "got: {out}");
    }

    #[test]
    fn malformed_profile_name_is_load_error() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
profiles:
  Prod:
    base_url: https://example.com
verifications: []
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(
            matches!(err, LoadError::MalformedProfileName { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn empty_profile_key_map_is_load_error() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
profiles:
  staging: {}
verifications: []
"#,
        );
        let err = load(&tmp.path().join("duhem.yml")).unwrap_err();
        assert!(matches!(err, LoadError::EmptyProfile { .. }), "got {err:?}");
    }

    #[test]
    fn well_formed_profile_loads() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
profiles:
  staging:
    base_url: https://staging.example.com
verifications:
  - path: ./a/duhem.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest { manifest, .. } => {
                assert!(manifest.profiles.contains_key("staging"));
            }
            _ => panic!("expected Manifest"),
        }
    }

    // ---- #69: manifest discovery (ancestor walk, candidate names) ----

    #[test]
    fn discover_explicit_file_returned_as_is() {
        let tmp = tempfile::tempdir().unwrap();
        let leaf = write(tmp.path(), "verification.yml", LEAF_A);
        // An explicit file path is used verbatim — Pattern A preserved.
        let resolved = discover(Some(&leaf), tmp.path()).unwrap();
        assert_eq!(resolved, leaf);
    }

    #[test]
    fn discover_cwd_directly_contains_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = write(
            tmp.path(),
            "duhem.yml",
            "manifest_version: 1\nverifications: []\n",
        );
        let resolved = discover(None, tmp.path()).unwrap();
        assert_eq!(resolved, manifest);
    }

    #[test]
    fn discover_walks_to_parent_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = write(
            tmp.path(),
            "duhem.yml",
            "manifest_version: 1\nverifications: []\n",
        );
        let sub = tmp.path().join("verifications").join("login");
        std::fs::create_dir_all(&sub).unwrap();
        // The cwd holds no manifest; the parent does → parent wins.
        let resolved = discover(None, &sub).unwrap();
        assert_eq!(resolved, manifest);
    }

    #[test]
    fn discover_closer_manifest_beats_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            "manifest_version: 1\nverifications: []\n",
        );
        let sub = tmp.path().join("sub");
        let closer = write(
            &sub,
            ".duhem.yml",
            "manifest_version: 1\nverifications: []\n",
        );
        // The cwd's `.duhem.yml` is closer than the parent's `duhem.yml`.
        let resolved = discover(None, &sub).unwrap();
        assert_eq!(resolved, closer);
    }

    #[test]
    fn discover_caps_at_git_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        // A `.git` directory at the repo root, no manifest anywhere.
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let err = discover(None, &sub).unwrap_err();
        match err {
            LoadError::ManifestNotFound { searched } => {
                // Probed the cwd and the `.git` root, then stopped — the
                // four candidate names in each of the two directories.
                assert_eq!(searched.len(), 8, "got {searched:?}");
                assert!(
                    searched.iter().all(|p| p.starts_with(tmp.path())),
                    "walk should not escape the repo boundary: {searched:?}"
                );
            }
            other => panic!("expected ManifestNotFound, got {other:?}"),
        }
    }

    #[test]
    fn discover_priority_prefers_dotless_yml() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical = write(
            tmp.path(),
            "duhem.yml",
            "manifest_version: 1\nverifications: []\n",
        );
        write(
            tmp.path(),
            ".duhem.yml",
            "manifest_version: 1\nverifications: []\n",
        );
        // Both present in the same dir → `duhem.yml` wins on priority.
        let resolved = discover(None, tmp.path()).unwrap();
        assert_eq!(resolved, canonical);
    }

    #[test]
    fn discover_explicit_dir_without_manifest_keeps_directory_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // Explicit directory with no manifest → today's error, not the
        // ancestor walk (explicit args behave identically to before).
        let err = discover(Some(tmp.path()), tmp.path()).unwrap_err();
        assert!(
            matches!(err, LoadError::DirectoryMissingManifest { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn defaults_block_round_trips_every_sub_key() {
        let y = r#"
manifest_version: 1
defaults:
  profile: staging
  timeout: 30s
  inconclusive_policy: warn
  retry:
    max: 2
    backoff: exponential
verifications: []
"#;
        let m = RootManifest::from_yaml_str(y).expect("parse");
        let d = m.defaults.as_ref().expect("defaults present");
        assert_eq!(d.profile.as_deref(), Some("staging"));
        assert_eq!(
            std::time::Duration::from(d.timeout.unwrap()),
            std::time::Duration::from_secs(30)
        );
        assert_eq!(d.inconclusive_policy, Some(InconclusivePolicy::Warn));
        let r = d.retry.as_ref().expect("retry present");
        assert_eq!(r.max, 2);
        assert_eq!(r.backoff, RetryBackoff::Exponential);
    }

    #[test]
    fn defaults_sub_keys_are_each_optional() {
        // A `defaults:` block may declare any subset of its sub-keys;
        // the absent ones deserialize to `None` (engine applies the
        // documented fallbacks).
        let y = r#"
manifest_version: 1
defaults:
  timeout: 10s
verifications: []
"#;
        let m = RootManifest::from_yaml_str(y).expect("parse");
        let d = m.defaults.unwrap();
        assert!(d.profile.is_none());
        assert!(d.inconclusive_policy.is_none());
        assert!(d.retry.is_none());
        assert!(d.timeout.is_some());
    }

    #[test]
    fn retry_max_defaults_to_zero_and_backoff_to_exponential() {
        let y = r#"
manifest_version: 1
defaults:
  retry: {}
verifications: []
"#;
        let m = RootManifest::from_yaml_str(y).expect("parse");
        let r = m.defaults.unwrap().retry.unwrap();
        assert_eq!(r.max, 0);
        assert_eq!(r.backoff, RetryBackoff::Exponential);
    }

    #[test]
    fn defaults_rejects_unknown_sub_key() {
        let y = r#"
manifest_version: 1
defaults:
  timeout: 30s
  bogus: 1
verifications: []
"#;
        assert!(RootManifest::from_yaml_str(y).is_err());
    }

    #[test]
    fn inconclusive_policy_rejects_unknown_variant() {
        let y = r#"
manifest_version: 1
defaults:
  inconclusive_policy: maybe
verifications: []
"#;
        assert!(RootManifest::from_yaml_str(y).is_err());
    }

    #[test]
    fn retry_backoff_rejects_unknown_variant() {
        let y = r#"
manifest_version: 1
defaults:
  retry:
    max: 1
    backoff: fibonacci
verifications: []
"#;
        assert!(RootManifest::from_yaml_str(y).is_err());
    }

    #[test]
    fn absent_defaults_round_trips_without_a_defaults_key() {
        // Additive guarantee: a manifest with no `defaults:` block must
        // serialize byte-for-byte as before — no `defaults` key leaks.
        let y = "manifest_version: 1\nverifications: []\n";
        let m = RootManifest::from_yaml_str(y).expect("parse");
        assert!(m.defaults.is_none());
        let out = serde_yml::to_string(&m).unwrap();
        assert!(!out.contains("defaults"), "got: {out}");
    }

    #[test]
    fn manifest_with_defaults_loads_through_load_pipeline() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
defaults:
  timeout: 15s
  inconclusive_policy: block
  retry:
    max: 1
    backoff: linear
verifications:
  - path: ./a/duhem.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest { manifest, .. } => {
                let d = manifest.defaults.expect("defaults present");
                assert_eq!(d.inconclusive_policy, Some(InconclusivePolicy::Block));
                assert_eq!(d.retry.unwrap().backoff, RetryBackoff::Linear);
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn glob_dedups_with_explicit_path() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
  - glob: ./*/duhem.yml
"#,
        );
        let loaded = load(&tmp.path().join("duhem.yml")).unwrap();
        match loaded {
            Loaded::Manifest { leaves, .. } => assert_eq!(leaves.len(), 1, "dedup repeated leaf"),
            _ => panic!("expected Manifest"),
        }
    }

    // ---- #384: leaf-by-path resolves through the root manifest ------
    //
    // Reference-detection and manifest-composition coverage lives in
    // `leaf_context::tests` (the module that owns that logic). What
    // stays here is `load_for_run` dispatch behavior: parity with
    // `load` on a manifest target, and the leaf-named-`duhem.yml`
    // self-reference edge case.

    #[test]
    fn load_for_run_manifest_target_matches_plain_load() {
        // #384 requirement: "do not change the no-path branch's
        // behaviour at all" — when the target already resolves to a
        // manifest, `load_for_run` takes the exact same branch `load`
        // does.
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a/duhem.yml", LEAF_A);
        let manifest = write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
verifications:
  - path: ./a/duhem.yml
"#,
        );
        let via_load = load(&manifest).unwrap();
        let via_run = load_for_run(&manifest).unwrap();
        match (via_load, via_run) {
            (Loaded::Manifest { leaves: a, .. }, Loaded::Manifest { leaves: b, .. }) => {
                assert_eq!(a.len(), b.len());
                assert_eq!(a[0].definition.verification, b[0].definition.verification);
            }
            other => panic!("expected both Manifest, got {other:?}"),
        }
    }

    #[test]
    fn load_for_run_leaf_named_duhem_yml_is_not_mistaken_for_its_own_manifest() {
        // Regression: Pattern A conventionally names a self-contained
        // leaf `duhem.yml` (it's exactly what `duhem init` scaffolds).
        // That name is also one of `MANIFEST_CANDIDATES`, so starting
        // the ancestor walk at the leaf's own directory must not
        // "find" the leaf as its own manifest and try to parse a
        // Verification Definition as a `RootManifest`.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let leaf = write(tmp.path(), "v/duhem.yml", LEAF_A);

        let loaded = load_for_run(&leaf).expect("a leaf must never resolve to itself");
        match loaded {
            Loaded::Leaf { definition, .. } => {
                assert_eq!(definition.verification, "leaf-a");
            }
            _ => panic!("expected Leaf"),
        }
    }

    #[test]
    fn load_for_run_leaf_named_duhem_yml_still_finds_a_real_ancestor_manifest() {
        // Companion to the regression above: excluding the leaf itself
        // must not blind the walk to a *real* manifest further up —
        // only the leaf's own path is skipped.
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "duhem.yml",
            r#"
manifest_version: 1
pages:
  login:
    heading: { text: Sign in }
verifications:
  - path: v/duhem.yml
"#,
        );
        let leaf = write(
            tmp.path(),
            "v/duhem.yml",
            r#"
verification: leaf-a
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/assert-element
            with:
              locator: $pages.login.heading
              expected: visible
"#,
        );

        let loaded = load_for_run(&leaf).expect("resolves through the real ancestor manifest");
        match loaded {
            Loaded::Leaf { definition, .. } => {
                assert!(
                    definition
                        .pages
                        .get("login")
                        .is_some_and(|e| e.contains_key("heading")),
                    "expected the ancestor manifest's page to merge in: {:?}",
                    definition.pages
                );
            }
            _ => panic!("expected Leaf"),
        }
    }
}
