//! `duhem` — the command-line entry point.
//!
//! Phase-0 skeleton. The free CLI binary (`docs/duhem-spec.md` §13)
//! offers `init`, `validate`, and `run`. `init` (issue #48)
//! scaffolds a runnable Verification Definition skeleton; the `run`
//! subcommand carries the authoring-ergonomics surface
//! (`--filter`, `--headed`, `--evidence-dir`, `--reporter`) per the
//! spec on issue #23. None of the `run` flags are correctness
//! gates — they make iteration on Verification Definitions
//! practical.

mod filter;
mod init;
mod inputs;
mod reporter;
mod reporter_config;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use duhem_actions::RunBrowser;
use duhem_judge::{RunVerdict, VerdictState, aggregate_run_set};
use duhem_runtime::{Engine, RunOutcome};
use duhem_schema::{
    InputDecl, InputType, Loaded, LoadedLeaf, VerificationDefinition, load as load_definition,
    validate,
};

use crate::filter::CliCheckFilter;
use crate::reporter::Reporter;
use crate::reporter_config::PluginRegistry;

/// Duhem — holistic verification for AI-delivered software.
#[derive(Debug, Parser)]
#[command(name = "duhem", version = VERSION_STRING, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

/// `--version` output. Bakes the Verification Definition schema version
/// in alongside the CLI version so authors can see at a glance which
/// schema `duhem validate` / `duhem run` will parse against.
const VERSION_STRING: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (schema v",
    duhem_schema::schema_version!(),
    ")"
);

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Scaffold a runnable Verification Definition skeleton.
    ///
    /// Produces a minimal, schema-valid Verification Definition the
    /// author can mutate toward their real workload. Deterministic
    /// and offline — AI-assisted generation is the Phase 1
    /// `duhem author` command (spec `docs/duhem-spec.md` §14), not
    /// this one. Spec on issue #48.
    Init {
        /// Target directory. Defaults to `./verifications/<name>/`
        /// when omitted.
        path: Option<PathBuf>,
        /// File-organization pattern from `docs/duhem-spec.md` §10.4.
        /// `A` (default) emits a single-file VD; `B` co-locates the
        /// VD under a sibling root manifest (stub until the manifest
        /// loader spec lands).
        #[arg(long = "pattern", value_name = "A|B", default_value = "A")]
        pattern: String,
        /// Verification slug, e.g. `dashboard-create-project`. Used
        /// for the default target dir and the `verification:` field
        /// in the generated YAML. Required on non-TTY stdin; prompted
        /// otherwise.
        #[arg(long = "name", value_name = "SLUG")]
        name: Option<String>,
        /// Overwrite a non-empty target. Without this, init exits 2
        /// and names the conflicting paths.
        #[arg(long = "force", default_value_t = false)]
        force: bool,
    },
    /// Parse and structurally validate a Verification Definition file.
    Validate {
        /// Path to a `.yml` Verification Definition.
        path: PathBuf,
    },
    /// Execute a Verification Definition end-to-end.
    ///
    /// `--filter`, `--headed`, `--evidence-dir`, `--reporter` are
    /// authoring-ergonomics flags from the spec on issue #23; none
    /// change the verdict on a non-filtered run.
    Run {
        /// Path to a `.yml` Verification Definition.
        path: PathBuf,
        /// `key=value` inputs, repeatable.
        #[arg(long = "inputs", value_name = "KEY=VALUE")]
        inputs: Vec<String>,
        /// Limit the run to a subset of `(criterion, check)` pairs.
        ///
        /// Grammar: `AC-1` (every check under `AC-1`),
        /// `AC-1::AC-1.2` (one pair), `AC-*::AC-*.1` (globbed). Repeat
        /// the flag to OR patterns: `--filter AC-1 --filter AC-2`.
        #[arg(long = "filter", value_name = "PATTERN")]
        filter: Vec<String>,
        /// Launch the browser with a visible window. Default is
        /// headless. Has no effect on `api/*` actions.
        #[arg(long = "headed", default_value_t = false)]
        headed: bool,
        /// Directory under which `EvidenceWriter` creates the per-run
        /// trace. Falls back to the engine default (`.duhem/runs`)
        /// when absent; created if missing.
        #[arg(long = "evidence-dir", value_name = "PATH")]
        evidence_dir: Option<PathBuf>,
        /// Stdout formatting for the post-run summary. Built-in
        /// names: `default` (verdict line), `quiet` (exit code
        /// only), `json` (one-line `RunSummary`). Other names are
        /// resolved against `.duhem.toml` (repo) + `~/.duhem/config.toml`
        /// (user) per the reporter-plugin spec on issue #34. Built-ins
        /// always win over a same-named plugin entry.
        #[arg(long = "reporter", value_name = "NAME", default_value = "default")]
        reporter: String,
        /// YAML or JSON file of `key: value` input pairs. Merged with
        /// any `--inputs k=v` flags; explicit `--inputs` always wins
        /// on the same key (spec on issue #33).
        #[arg(long = "inputs-file", value_name = "PATH")]
        inputs_file: Option<PathBuf>,
        /// Parse + validate the definition, resolve the filter, print
        /// the `(criterion::check)` pairs that *would* run, and exit
        /// 0 without launching the browser or writing evidence. Use
        /// when authoring a Verification Definition to confirm a
        /// `--filter` resolves to the pairs you expect (spec on #33).
        #[arg(long = "dry-run", default_value_t = false)]
        dry_run: bool,
        /// Seed for the runtime's entropy source. With a seed set,
        /// `$runtime.uuid()` is derived deterministically from the
        /// seed (two runs with the same seed see the same uuid
        /// string). Run IDs and event timestamps are not seeded, so
        /// `trace.jsonl` is not byte-identical across runs; the
        /// guarantee is over evaluator-visible entropy. Spec on
        /// issue #33.
        #[arg(long = "seed", value_name = "U64")]
        seed: Option<u64>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        None => ExitCode::SUCCESS,
        Some(Cmd::Init {
            path,
            pattern,
            name,
            force,
        }) => run_init(path, &pattern, name, force),
        Some(Cmd::Validate { path }) => match run_validate(&path) {
            Ok(()) => {
                println!("OK");
                ExitCode::SUCCESS
            }
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        },
        Some(Cmd::Run {
            path,
            inputs,
            filter,
            headed,
            evidence_dir,
            reporter,
            inputs_file,
            dry_run,
            seed,
        }) => {
            // Resolve the reporter name BEFORE we boot the tokio
            // runtime / browser: a typoed `--reporter` should exit 2
            // with `unknown reporter:` on stderr without any work
            // happening first.
            //
            // Try built-ins first so a malformed `.duhem.toml` or
            // `~/.duhem/config.toml` doesn't break `--reporter
            // default`/`quiet`/`json` — the documented resolution
            // order has built-ins winning before any config is read.
            let resolved_reporter = match reporter::resolve_built_in(&reporter) {
                Some(r) => r,
                None => {
                    let registry = match PluginRegistry::load() {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("{e}");
                            return ExitCode::FAILURE;
                        }
                    };
                    match reporter::resolve_plugin(&reporter, &registry) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("{e}");
                            // Spec on #34 Test § "unknown name yields exit 2".
                            return ExitCode::from(2);
                        }
                    }
                }
            };
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(run_command(RunArgs {
                path,
                inputs,
                filter,
                headed,
                evidence_dir,
                reporter: resolved_reporter,
                inputs_file,
                dry_run,
                seed,
            }))
        }
    }
}

/// Resolved `duhem run` arguments. Kept as a struct so the dispatch
/// function's signature doesn't grow unbounded as new flags land.
struct RunArgs {
    path: PathBuf,
    inputs: Vec<String>,
    filter: Vec<String>,
    headed: bool,
    evidence_dir: Option<PathBuf>,
    reporter: Reporter,
    inputs_file: Option<PathBuf>,
    dry_run: bool,
    seed: Option<u64>,
}

/// `duhem init` dispatch. Wraps `init::run` with the CLI-level
/// translation: parse `--pattern`, map outcomes to the three exit
/// codes the spec defines (0 success / 2 conflict / 3 warning),
/// and print post-init guidance.
fn run_init(path: Option<PathBuf>, pattern: &str, name: Option<String>, force: bool) -> ExitCode {
    let pattern: init::Pattern = match pattern.parse() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let args = init::InitArgs {
        path,
        pattern,
        name,
        force,
    };
    match init::run(args) {
        Ok(outcome) => {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "Created:");
            for p in &outcome.created {
                let _ = writeln!(stdout, "  {}", p.display());
            }
            // The per-feature `duhem.yml` is always the first entry
            // `init::run` pushes (Pattern A: only one; Pattern B:
            // pushed before the parent root manifest). Use that as
            // the next-command pointer.
            let vd_path = outcome
                .created
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("<verification>/duhem.yml"));
            let _ = writeln!(stdout, "\nNext:");
            let _ = writeln!(stdout, "  duhem validate {}", vd_path.display());
            let _ = writeln!(stdout, "  duhem run      {}", vd_path.display());
            let _ = writeln!(
                stdout,
                "\nAuthoring guide: .claude/skills/verification-authoring/SKILL.md"
            );
            let _ = stdout.flush();

            if outcome.warnings.is_empty() {
                ExitCode::SUCCESS
            } else {
                for w in &outcome.warnings {
                    eprintln!("warning: {w}");
                }
                // Spec § Plan: "exit with a non-zero warning code
                // (distinct from the error code)." Reserve 2 for the
                // conflict-refusal error path; use 3 for warnings.
                ExitCode::from(3)
            }
        }
        Err(e @ init::InitError::ExistingNonEmpty { .. }) => {
            eprintln!("{e}");
            // Spec § Design: existing non-empty target without
            // --force exits 2.
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run_validate(path: &std::path::Path) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let v = VerificationDefinition::from_yaml_str(&src).map_err(|e| match e.location() {
        Some(loc) => format!(
            "{}:{}:{}: [schema v{}] {e}",
            path.display(),
            loc.line(),
            loc.column(),
            duhem_schema::SCHEMA_VERSION
        ),
        None => format!(
            "{}: [schema v{}] {e}",
            path.display(),
            duhem_schema::SCHEMA_VERSION
        ),
    })?;
    validate(&v).map_err(|errs| {
        let plural = if errs.len() == 1 { "" } else { "s" };
        // Preamble names the schema version the file was validated
        // against — when authors hit a validation error, the next
        // question is "which schema?", and a downstream VD that pinned
        // a different version needs to see the mismatch.
        let mut s = format!(
            "[schema v{}] {} validation error{plural}:",
            duhem_schema::SCHEMA_VERSION,
            errs.len()
        );
        for e in errs {
            s.push_str("\n  - ");
            s.push_str(&e.to_string());
        }
        s
    })
}

async fn run_command(args: RunArgs) -> ExitCode {
    let RunArgs {
        path,
        inputs: raw_inputs,
        filter: raw_filter,
        headed,
        evidence_dir,
        reporter,
        inputs_file,
        dry_run,
        seed,
    } = args;

    // Polymorphic load: directory → `<dir>/duhem.yml`; manifest →
    // expand leaves; leaf → single Verification Definition (today's
    // behavior). Spec on issue #49. The loader annotates YAML / shape
    // failures with the offending path; we prefix the schema version
    // so authors see at a glance which schema the loader parsed
    // against (spec on #51).
    let loaded = match load_definition(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[schema v{}] {e}", duhem_schema::SCHEMA_VERSION);
            return ExitCode::FAILURE;
        }
    };

    // Load `--inputs-file` before resolving so file values participate
    // in the same required/unknown/typed checks as explicit flags.
    // Inputs apply to every leaf the manifest expands to — per the
    // issue, the manifest does not remap inputs per leaf in v1.
    let file_inputs = match inputs_file.as_deref() {
        Some(p) => match inputs::load_inputs_file(p) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        },
        None => BTreeMap::new(),
    };

    // Filter parse failures must surface before we boot a browser —
    // a typoed pattern shouldn't pay the Playwright launch cost. The
    // manifest case also benefits: a typo blocks every leaf, not the
    // first one to spin up.
    let check_filter = if raw_filter.is_empty() {
        None
    } else {
        match CliCheckFilter::parse(&raw_filter) {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    };

    // Normalize the load into a list of `(leaf_name, leaf_path, def)`
    // tuples plus the evidence-namespacing strategy. A single leaf
    // stays in the today's evidence layout (`<root>/<run_id>/`); a
    // manifest namespaces per-leaf (`<root>/<leaf>/<run_id>/`).
    enum Scope {
        SingleLeaf,
        Manifest { warnings: Vec<String> },
    }
    let (leaves, scope): (Vec<LoadedLeaf>, Scope) = match loaded {
        Loaded::Leaf { path, definition } => {
            (vec![LoadedLeaf { path, definition }], Scope::SingleLeaf)
        }
        Loaded::Manifest {
            leaves, warnings, ..
        } => (leaves, Scope::Manifest { warnings }),
    };
    if let Scope::Manifest { warnings } = &scope {
        for w in warnings {
            eprintln!("warning: {w}");
        }
    }
    let is_manifest = matches!(scope, Scope::Manifest { .. });

    // Validate + resolve inputs for every leaf up front, before any
    // browser launch. A malformed leaf in a manifest should not
    // produce a half-run; the loader already fails the load on a
    // YAML-parse leaf failure, this catches structural validation.
    let mut resolved: Vec<(
        String,
        std::path::PathBuf,
        VerificationDefinition,
        BTreeMap<String, serde_json::Value>,
    )> = Vec::with_capacity(leaves.len());
    for leaf in &leaves {
        if let Err(errs) = validate(&leaf.definition) {
            let plural = if errs.len() == 1 { "" } else { "s" };
            eprintln!(
                "{}: [schema v{}] {} validation error{plural}:",
                leaf.path.display(),
                duhem_schema::SCHEMA_VERSION,
                errs.len()
            );
            for e in errs {
                eprintln!("  - {e}");
            }
            return ExitCode::FAILURE;
        }
        let inputs = match resolve_inputs(&raw_inputs, &file_inputs, &leaf.definition.inputs) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("{}: {e}", leaf.path.display());
                return ExitCode::FAILURE;
            }
        };
        let name = leaf_name(&leaf.path);
        resolved.push((name, leaf.path.clone(), leaf.definition.clone(), inputs));
    }

    // `--dry-run` short-circuits before any browser launch: print the
    // resolved `(criterion::check)` plan (qualified with the
    // verification name on manifest runs) and exit 0.
    if dry_run {
        let mut stdout = std::io::stdout().lock();
        let mut wrote = false;
        for (name, _path, def, _inputs) in &resolved {
            let leaf_filter = check_filter.as_ref().and_then(|f| f.for_verification(name));
            // If a filter was passed and nothing scopes to this leaf,
            // skip — no spurious "no checks matched" line per-leaf.
            if check_filter.is_some() && leaf_filter.is_none() {
                continue;
            }
            for criterion in &def.criteria {
                for check in &criterion.checks {
                    let matched = match &leaf_filter {
                        None => true,
                        Some(f) => {
                            use duhem_runtime::CheckFilter;
                            f.matches(&criterion.id, &check.id)
                        }
                    };
                    if matched {
                        let line = if is_manifest {
                            format!("WOULD RUN: {}::{}::{}", name, criterion.id, check.id)
                        } else {
                            format!("WOULD RUN: {}::{}", criterion.id, check.id)
                        };
                        if let Err(e) = writeln!(stdout, "{line}") {
                            eprintln!("dry-run: {e}");
                            return ExitCode::FAILURE;
                        }
                        wrote = true;
                    }
                }
            }
        }
        if !wrote && let Err(e) = writeln!(stdout, "WOULD RUN: (no checks matched filter)") {
            eprintln!("dry-run: {e}");
            return ExitCode::FAILURE;
        }
        if let Err(e) = stdout.flush() {
            eprintln!("dry-run: {e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    let mut leaf_outcomes: Vec<(String, RunOutcome)> = Vec::with_capacity(resolved.len());
    for (name, leaf_path, def, inputs) in resolved {
        // Per-leaf filter: every leaf is narrowed by name regardless
        // of `is_manifest`, so a `<verification>::<criterion>::<check>`
        // pattern behaves identically against a single leaf and a
        // manifest leaf (Copilot PR #60 review). On a manifest, an
        // empty post-narrow filter means "skip this leaf entirely";
        // on a single leaf, it falls through to the engine as an
        // empty filter so the run produces the same empty-aggregation
        // signal a typo'd `--filter` would on any leaf — consistent
        // with `--dry-run` which already prints
        // `(no checks matched filter)` for the same case.
        let leaf_filter = match check_filter.as_ref() {
            Some(f) => {
                let narrowed = f.for_verification(&name);
                match (narrowed, is_manifest) {
                    (Some(n), _) => Some(n),
                    (None, true) => continue,
                    (None, false) => Some(CliCheckFilter::matches_nothing()),
                }
            }
            None => None,
        };

        // One browser per leaf. Phase-0 leaves run serially (#49) and
        // `RunBrowser` is non-`Clone`, so the cleanest model is a
        // fresh launch per leaf — same per-leaf isolation we'd want
        // even after we have a sharable handle.
        let browser = match RunBrowser::launch(headed).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("browser: {e}");
                return ExitCode::FAILURE;
            }
        };

        let mut engine = Engine::new()
            .with_browser(browser)
            .with_definition_path(leaf_path.display().to_string());
        let root = match (evidence_dir.as_ref(), is_manifest) {
            (Some(dir), true) => dir.join(&name),
            (Some(dir), false) => dir.clone(),
            (None, true) => PathBuf::from(".duhem/runs").join(&name),
            (None, false) => PathBuf::from(".duhem/runs"),
        };
        engine = engine.with_evidence_root(root);
        if let Some(f) = leaf_filter {
            engine = engine.with_filter(f);
        }
        if let Some(s) = seed {
            engine = engine.with_seed(s);
        }
        let outcome = match engine.run_with_metadata(&def, inputs).await {
            Ok(o) => o,
            Err(e) => {
                eprintln!("engine ({}): {e}", leaf_path.display());
                return ExitCode::FAILURE;
            }
        };
        leaf_outcomes.push((name, outcome));
    }

    // Reporter rendering:
    //
    // - Single leaf: today's behavior — one `render(reporter, outcome)`
    //   call.
    // - Manifest: per-leaf invocation of the same reporter, plus a
    //   top-level aggregated verdict via `render_set`. Plugin
    //   reporters that don't yet understand `RunSetSummary` continue
    //   to work as before because they see one `RunSummary` per leaf
    //   (issue #49: "default no-op so existing reporters compile
    //   unchanged"); the set-level summary is the CLI's own concern.
    let mut stdout = std::io::stdout().lock();
    if !is_manifest {
        let (_, outcome) = &leaf_outcomes[0];
        if let Err(e) = reporter::render(&reporter, &mut stdout, outcome) {
            eprintln!("reporter: {e}");
            return ExitCode::FAILURE;
        }
        if let Err(e) = stdout.flush() {
            eprintln!("reporter: {e}");
            return ExitCode::FAILURE;
        }
        return match outcome.verdict.state {
            VerdictState::Pass => ExitCode::SUCCESS,
            _ => ExitCode::FAILURE,
        };
    }

    // Manifest path: aggregate verdicts and render the set.
    let run_verdicts: Vec<RunVerdict> = leaf_outcomes
        .iter()
        .map(|(_, o)| o.verdict.clone())
        .collect();
    let set_verdict = aggregate_run_set(run_verdicts);
    if let Err(e) = reporter::render_set(&reporter, &mut stdout, &leaf_outcomes, &set_verdict) {
        eprintln!("reporter: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = stdout.flush() {
        eprintln!("reporter: {e}");
        return ExitCode::FAILURE;
    }

    match set_verdict.state {
        VerdictState::Pass => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// Derive the canonical "verification name" of a leaf for evidence
/// namespacing and `--filter` matching.
///
/// The leaf file's *name* drives the choice:
///
/// - `duhem.yml` (the §10.4 Pattern B / C layout) → the parent
///   directory name. This is the case where the parent dir is the
///   real feature identifier and the file is generic.
/// - any other filename (e.g. `verifications/create-workspace.yml`)
///   → the file stem. Falling through to the parent dir name here
///   would collapse every sibling leaf to the same name and break
///   per-leaf evidence isolation (Copilot PR #60 review).
///
/// Empty / `.` / `..` parent segments defeat the parent-dir signal,
/// in which case we always fall back to the file stem.
fn leaf_name(path: &std::path::Path) -> String {
    let file_name = path.file_name().and_then(|n| n.to_str());
    let is_duhem_yml = matches!(file_name, Some("duhem.yml") | Some("duhem.yaml"));
    if is_duhem_yml
        && let Some(parent) = path.parent()
        && let Some(name) = parent.file_name().and_then(|n| n.to_str())
        && !name.is_empty()
        && name != "."
        && name != ".."
    {
        return name.to_string();
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("leaf")
        .to_string()
}

#[cfg(test)]
mod leaf_name_tests {
    use super::leaf_name;
    use std::path::PathBuf;

    #[test]
    fn duhem_yml_uses_parent_dir_name() {
        // Pattern B layout: every leaf is `duhem.yml` and the dir
        // around it carries the feature identifier.
        assert_eq!(leaf_name(&PathBuf::from("leaf-a/duhem.yml")), "leaf-a");
        assert_eq!(
            leaf_name(&PathBuf::from("verifications/login/duhem.yml")),
            "login"
        );
        assert_eq!(leaf_name(&PathBuf::from("duhem.yaml")), "duhem");
    }

    #[test]
    fn bare_yml_uses_file_stem() {
        // Pattern C with named files: sibling leaves share a parent
        // dir, so the file stem is the only thing that disambiguates
        // them. Returning the parent dir name would collide.
        assert_eq!(
            leaf_name(&PathBuf::from("verifications/login.yml")),
            "login"
        );
        assert_eq!(
            leaf_name(&PathBuf::from("verifications/create-workspace.yml")),
            "create-workspace"
        );
    }

    #[test]
    fn sibling_named_leaves_get_distinct_names() {
        // Direct regression check for the bug Copilot flagged on the
        // pre-fix heuristic: two sibling leaves under the same parent
        // dir must not collapse to the same evidence namespace.
        let a = leaf_name(&PathBuf::from("verifications/login.yml"));
        let b = leaf_name(&PathBuf::from("verifications/signup.yml"));
        assert_ne!(a, b);
    }

    #[test]
    fn no_parent_dir_falls_back_to_stem() {
        assert_eq!(leaf_name(&PathBuf::from("leaf.yml")), "leaf");
    }
}

/// Parse the raw `--inputs k=v` flags into a `(name, raw)` map.
fn parse_inputs(raw: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for s in raw {
        let (k, v) = s
            .split_once('=')
            .ok_or_else(|| format!("--inputs `{s}`: expected `key=value`"))?;
        if k.is_empty() {
            return Err(format!("--inputs `{s}`: empty key"));
        }
        out.insert(k.to_string(), v.to_string());
    }
    Ok(out)
}

/// Resolve `--inputs k=v` flags + an optional `--inputs-file` map
/// against the Verification Definition's `inputs:` block. Per the
/// typed-input-catalog spec plus #33:
///
/// - Unknown input (in either source) → error.
/// - Explicit `--inputs k=v` value → coerced per declared `InputType`.
///   Always wins over a same-key file value.
/// - File value → validated against declared `InputType` (the file's
///   parser already produced typed JSON; we only need to confirm shape).
/// - Not provided in either source + default present → default carried
///   through as-is (the schema validator type-checked it at parse time).
/// - Not provided in either source + no default → error.
fn resolve_inputs(
    raw: &[String],
    file: &BTreeMap<String, serde_json::Value>,
    decls: &BTreeMap<String, InputDecl>,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let provided = parse_inputs(raw)?;
    for name in provided.keys() {
        if !decls.contains_key(name) {
            return Err(format!("unknown input: `{name}`"));
        }
    }
    for name in file.keys() {
        if !decls.contains_key(name) {
            return Err(format!("unknown input (from --inputs-file): `{name}`"));
        }
    }
    let mut out = BTreeMap::new();
    for (name, decl) in decls {
        if let Some(raw_value) = provided.get(name) {
            let coerced = coerce_input(name, decl.kind, raw_value)?;
            out.insert(name.clone(), coerced);
        } else if let Some(file_value) = file.get(name) {
            validate_file_value(name, decl.kind, file_value)?;
            out.insert(name.clone(), file_value.clone());
        } else if let Some(default) = &decl.default {
            let value =
                yml_to_json(default).map_err(|e| format!("input `{name}`: default: {e}"))?;
            out.insert(name.clone(), value);
        } else {
            return Err(format!("missing required input: `{name}`"));
        }
    }
    Ok(out)
}

/// Type-check a value loaded from `--inputs-file` against its declared
/// `InputType`. The file's parser already gave us a typed JSON value,
/// so this is a shape check, not a string coercion. Mirrors the
/// promotion rule used by the schema validator: an `integer` is a
/// valid `number`, but not vice versa.
fn validate_file_value(name: &str, kind: InputType, v: &serde_json::Value) -> Result<(), String> {
    let actual = json_shape_name(v);
    let ok = match kind {
        InputType::String => matches!(v, serde_json::Value::String(_)),
        InputType::Integer => v.as_i64().is_some(),
        InputType::Number => v.is_number(),
        InputType::Boolean => matches!(v, serde_json::Value::Bool(_)),
        InputType::Array => matches!(v, serde_json::Value::Array(_)),
        InputType::Object => matches!(v, serde_json::Value::Object(_)),
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "input `{name}` (from --inputs-file): expected {kind}, got {actual}"
        ))
    }
}

/// Coerce a `--inputs k=v` value to its declared `InputType`. Failure
/// surfaces as a CLI-friendly error naming the input and the expected
/// type.
fn coerce_input(name: &str, kind: InputType, v: &str) -> Result<serde_json::Value, String> {
    match kind {
        InputType::String => Ok(serde_json::Value::String(v.to_string())),
        InputType::Integer => v
            .parse::<i64>()
            .map(|n| serde_json::Value::Number(n.into()))
            .map_err(|_| format!("--inputs `{name}={v}`: expected integer, got `{v}`")),
        InputType::Number => {
            // Accept integer literals as `number`; serde_json picks the
            // narrowest representation. Fractional values stay
            // fractional.
            if let Ok(i) = v.parse::<i64>() {
                Ok(serde_json::Value::Number(i.into()))
            } else if let Ok(f) = v.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| {
                        format!("--inputs `{name}={v}`: number not representable as f64")
                    })
            } else {
                Err(format!("--inputs `{name}={v}`: expected number, got `{v}`"))
            }
        }
        InputType::Boolean => match v {
            // Strict per Alignment §"Boolean strictness at the CLI":
            // only the canonical `true` / `false` literals.
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            _ => Err(format!(
                "--inputs `{name}={v}`: expected boolean (`true` or `false`), got `{v}`"
            )),
        },
        InputType::Array => {
            let parsed: serde_json::Value = serde_json::from_str(v).map_err(|e| {
                format!("--inputs `{name}={v}`: expected JSON array, parse error: {e}")
            })?;
            if !parsed.is_array() {
                return Err(format!(
                    "--inputs `{name}={v}`: expected JSON array, got {}",
                    json_shape_name(&parsed)
                ));
            }
            Ok(parsed)
        }
        InputType::Object => {
            let parsed: serde_json::Value = serde_json::from_str(v).map_err(|e| {
                format!("--inputs `{name}={v}`: expected JSON object, parse error: {e}")
            })?;
            if !parsed.is_object() {
                return Err(format!(
                    "--inputs `{name}={v}`: expected JSON object, got {}",
                    json_shape_name(&parsed)
                ));
            }
            Ok(parsed)
        }
    }
}

fn json_shape_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Convert a YAML default value into JSON for engine consumption.
///
/// Fallible because YAML permits non-string mapping keys (e.g.
/// `default: { 1: "x" }`); JSON does not. Silently dropping such
/// entries would mutate the author's default; we surface them as a
/// user-facing error instead.
fn yml_to_json(v: &serde_yml::Value) -> Result<serde_json::Value, String> {
    use serde_yml::Value as Y;
    Ok(match v {
        Y::Null => serde_json::Value::Null,
        Y::Bool(b) => serde_json::Value::Bool(*b),
        Y::Number(n) => serde_json::to_value(n).unwrap_or(serde_json::Value::Null),
        Y::String(s) => serde_json::Value::String(s.clone()),
        Y::Sequence(seq) => {
            let mut out = Vec::with_capacity(seq.len());
            for item in seq {
                out.push(yml_to_json(item)?);
            }
            serde_json::Value::Array(out)
        }
        Y::Mapping(m) => {
            let mut out = serde_json::Map::with_capacity(m.len());
            for (k, v) in m {
                let key = k.as_str().ok_or_else(|| {
                    "object default has a non-string mapping key (not representable as JSON)"
                        .to_string()
                })?;
                out.insert(key.to_string(), yml_to_json(v)?);
            }
            serde_json::Value::Object(out)
        }
        Y::Tagged(t) => yml_to_json(&t.value)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Spec on #23 § Alignment "Headless default": `--headed` opts
    /// into a visible window; default stays headless. The runtime
    /// `RunBrowser::launch(headed: bool)` is keyed off this exact
    /// boolean, so the CLI → launch-arg translation is the smallest
    /// thing we can validate without booting Playwright.
    #[test]
    fn headed_flag_defaults_to_false_and_opts_in() {
        let default = Cli::try_parse_from(["duhem", "run", "v.yml"]).expect("parse");
        match default.cmd {
            Some(Cmd::Run { headed, .. }) => assert!(!headed, "default is headless"),
            other => panic!("expected Run, got {other:?}"),
        }
        let opted = Cli::try_parse_from(["duhem", "run", "v.yml", "--headed"]).expect("parse");
        match opted.cmd {
            Some(Cmd::Run { headed, .. }) => assert!(headed, "--headed opts in"),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn reporter_flag_parses_as_free_string_and_defaults_to_default() {
        // After the reporter-plugin spec (#34) the `--reporter` flag
        // is a free string: built-ins (`default`/`quiet`/`json`) are
        // matched at runtime against the config registry. clap-level
        // we only verify the name is captured.
        let default = Cli::try_parse_from(["duhem", "run", "v.yml"]).expect("parse");
        match default.cmd {
            Some(Cmd::Run { reporter, .. }) => assert_eq!(reporter, "default"),
            _ => panic!("expected Run"),
        }
        for name in ["default", "quiet", "json", "pretty", "junit"] {
            let parsed =
                Cli::try_parse_from(["duhem", "run", "v.yml", "--reporter", name]).expect("parse");
            match parsed.cmd {
                Some(Cmd::Run { reporter, .. }) => assert_eq!(reporter, name, "for `{name}`"),
                _ => panic!("expected Run"),
            }
        }
    }

    #[test]
    fn filter_flag_collects_repeated_values() {
        // Spec on #23: repeat `--filter` for OR. Verify the CLI surface
        // collects into the expected `Vec<String>` shape.
        let parsed = Cli::try_parse_from([
            "duhem",
            "run",
            "v.yml",
            "--filter",
            "AC-1",
            "--filter",
            "AC-2::AC-2.3",
        ])
        .expect("parse");
        match parsed.cmd {
            Some(Cmd::Run { filter, .. }) => {
                assert_eq!(filter, vec!["AC-1".to_string(), "AC-2::AC-2.3".to_string()]);
            }
            _ => panic!("expected Run"),
        }
    }

    fn decls(yaml: &str) -> BTreeMap<String, InputDecl> {
        let y = format!("verification: x\ninputs:\n{yaml}\ncriteria: []\n");
        VerificationDefinition::from_yaml_str(&y)
            .expect("parse")
            .inputs
    }

    fn raw(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// Test-only shorthand: resolve with no `--inputs-file` source.
    /// The file-merge code path is exercised separately below.
    fn resolve(
        cli: &[String],
        decls: &BTreeMap<String, InputDecl>,
    ) -> Result<BTreeMap<String, serde_json::Value>, String> {
        let empty = BTreeMap::new();
        resolve_inputs(cli, &empty, decls)
    }

    #[test]
    fn coerces_integer_input() {
        let d = decls("  count: { type: integer }");
        let out = resolve(&raw(&["count=3"]), &d).expect("ok");
        assert_eq!(out["count"], serde_json::json!(3));
    }

    #[test]
    fn integer_rejects_non_numeric() {
        let d = decls("  count: { type: integer }");
        let err = resolve(&raw(&["count=foo"]), &d).unwrap_err();
        assert!(err.contains("count"), "error names the input: {err}");
        assert!(
            err.contains("integer"),
            "error names the expected type: {err}"
        );
    }

    #[test]
    fn integer_rejects_fractional() {
        let d = decls("  count: { type: integer }");
        let err = resolve(&raw(&["count=1.5"]), &d).unwrap_err();
        assert!(err.contains("count"), "error names the input: {err}");
    }

    #[test]
    fn number_accepts_fractional_and_integer() {
        let d = decls("  threshold: { type: number }");
        let frac = resolve(&raw(&["threshold=0.85"]), &d).unwrap();
        assert_eq!(frac["threshold"], serde_json::json!(0.85));
        let whole = resolve(&raw(&["threshold=1"]), &d).unwrap();
        assert_eq!(whole["threshold"], serde_json::json!(1));
    }

    #[test]
    fn boolean_accepts_only_true_or_false() {
        let d = decls("  flag: { type: boolean }");
        let t = resolve(&raw(&["flag=true"]), &d).unwrap();
        assert_eq!(t["flag"], serde_json::json!(true));
        let f = resolve(&raw(&["flag=false"]), &d).unwrap();
        assert_eq!(f["flag"], serde_json::json!(false));
        // `1` / `yes` are rejected per the Alignment §"Boolean
        // strictness" decision: shell ergonomics don't justify
        // ambiguous parses for a verifier.
        for bad in ["1", "0", "yes", "no", "True", "FALSE"] {
            let err = resolve(&raw(&[&format!("flag={bad}")]), &d).unwrap_err();
            assert!(err.contains("boolean"), "rejecting `{bad}`: {err}");
        }
    }

    #[test]
    fn string_takes_value_literally() {
        // String values are NOT JSON-parsed — `--inputs name=foo`
        // gives the literal `foo`, never the JSON parse of `foo`
        // (which would error).
        let d = decls("  name: { type: string }");
        let out = resolve(&raw(&["name=hello world"]), &d).unwrap();
        assert_eq!(out["name"], serde_json::json!("hello world"));
    }

    #[test]
    fn array_parses_as_json() {
        let d = decls("  roles: { type: array }");
        let out = resolve(&raw(&[r#"roles=["admin","viewer"]"#]), &d).unwrap();
        assert_eq!(out["roles"], serde_json::json!(["admin", "viewer"]));
    }

    #[test]
    fn array_rejects_object_json() {
        let d = decls("  roles: { type: array }");
        let err = resolve(&raw(&[r#"roles={"a":1}"#]), &d).unwrap_err();
        assert!(err.contains("array"), "error names expected type: {err}");
    }

    #[test]
    fn object_parses_as_json() {
        let d = decls("  flags: { type: object }");
        let out = resolve(&raw(&[r#"flags={"dark":true}"#]), &d).unwrap();
        assert_eq!(out["flags"], serde_json::json!({"dark": true}));
    }

    #[test]
    fn object_rejects_array_json() {
        let d = decls("  flags: { type: object }");
        let err = resolve(&raw(&[r#"flags=[1,2]"#]), &d).unwrap_err();
        assert!(err.contains("object"), "error names expected type: {err}");
    }

    #[test]
    fn missing_required_input_errors() {
        let d = decls("  count: { type: integer }");
        let err = resolve(&raw(&[]), &d).unwrap_err();
        assert!(
            err.contains("missing required input") && err.contains("count"),
            "got: {err}"
        );
    }

    #[test]
    fn unknown_input_errors() {
        let d = decls("  count: { type: integer }");
        let err = resolve(&raw(&["count=1", "bogus=3"]), &d).unwrap_err();
        assert!(
            err.contains("unknown input") && err.contains("bogus"),
            "got: {err}"
        );
    }

    #[test]
    fn declared_default_is_used_when_input_absent() {
        let d = decls("  name: { type: string, default: \"ws-default\" }");
        let out = resolve(&raw(&[]), &d).unwrap();
        assert_eq!(out["name"], serde_json::json!("ws-default"));
    }

    #[test]
    fn explicit_input_overrides_default() {
        let d = decls("  name: { type: string, default: \"ws-default\" }");
        let out = resolve(&raw(&["name=other"]), &d).unwrap();
        assert_eq!(out["name"], serde_json::json!("other"));
    }

    #[test]
    fn object_default_with_non_string_keys_errors() {
        // YAML allows non-string mapping keys; JSON does not. Silently
        // dropping such entries would mutate the author's default —
        // surface it as a user-facing error from `resolve_inputs`.
        let d = decls("  flags: { type: object, default: { 1: x } }");
        let err = resolve(&raw(&[]), &d).unwrap_err();
        assert!(
            err.contains("flags") && err.contains("non-string"),
            "error names the input and the cause: {err}"
        );
    }

    // ---- #33: --inputs-file merge / --dry-run / --seed CLI parsing ----

    /// Spec on #33 § Alignment "Conflict semantics between `--inputs`
    /// and `--inputs-file`": explicit `--inputs k=v` wins over a file
    /// value on the same key.
    #[test]
    fn explicit_inputs_override_inputs_file_on_same_key() {
        let d = decls("  base_url: { type: string }");
        let mut file = BTreeMap::new();
        file.insert("base_url".into(), serde_json::json!("from-file"));
        let out = resolve_inputs(&raw(&["base_url=from-flag"]), &file, &d).unwrap();
        assert_eq!(out["base_url"], serde_json::json!("from-flag"));
    }

    #[test]
    fn inputs_file_supplies_value_when_flag_absent() {
        let d = decls("  count: { type: integer }");
        let mut file = BTreeMap::new();
        file.insert("count".into(), serde_json::json!(7));
        let out = resolve_inputs(&raw(&[]), &file, &d).unwrap();
        assert_eq!(out["count"], serde_json::json!(7));
    }

    #[test]
    fn inputs_file_typed_value_validates_against_declared_type() {
        // A file value whose JSON shape doesn't match the declared
        // `InputType` is a real authoring error — surface it as a
        // CLI-side failure, not as a confusing runtime
        // `Inconclusive(TypeMismatch)` later.
        let d = decls("  count: { type: integer }");
        let mut file = BTreeMap::new();
        file.insert("count".into(), serde_json::json!("not a number"));
        let err = resolve_inputs(&raw(&[]), &file, &d).unwrap_err();
        assert!(
            err.contains("count") && err.contains("integer"),
            "got: {err}"
        );
    }

    #[test]
    fn unknown_input_in_file_is_an_error() {
        let d = decls("  count: { type: integer }");
        let mut file = BTreeMap::new();
        file.insert("bogus".into(), serde_json::json!(1));
        file.insert("count".into(), serde_json::json!(1));
        let err = resolve_inputs(&raw(&[]), &file, &d).unwrap_err();
        assert!(
            err.contains("--inputs-file") && err.contains("bogus"),
            "got: {err}"
        );
    }

    #[test]
    fn explicit_input_still_overrides_default_even_with_file() {
        // Resolution order: explicit `--inputs` > `--inputs-file` >
        // declared default > error. Confirm the precedence with all
        // three present.
        let d = decls("  name: { type: string, default: ws-default }");
        let mut file = BTreeMap::new();
        file.insert("name".into(), serde_json::json!("from-file"));
        let out = resolve_inputs(&raw(&["name=from-flag"]), &file, &d).unwrap();
        assert_eq!(out["name"], serde_json::json!("from-flag"));
    }

    #[test]
    fn dry_run_flag_parses_and_defaults_false() {
        let default = Cli::try_parse_from(["duhem", "run", "v.yml"]).expect("parse");
        match default.cmd {
            Some(Cmd::Run { dry_run, .. }) => assert!(!dry_run, "default off"),
            _ => panic!("expected Run"),
        }
        let opted = Cli::try_parse_from(["duhem", "run", "v.yml", "--dry-run"]).expect("parse");
        match opted.cmd {
            Some(Cmd::Run { dry_run, .. }) => assert!(dry_run, "--dry-run opts in"),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn seed_flag_parses_as_u64() {
        let parsed = Cli::try_parse_from(["duhem", "run", "v.yml", "--seed", "42"]).expect("parse");
        match parsed.cmd {
            Some(Cmd::Run { seed, .. }) => assert_eq!(seed, Some(42)),
            _ => panic!("expected Run"),
        }
        // Negative / non-numeric seed rejected by clap's u64 parser:
        // protects authors from accidentally passing an option-looking
        // arg that silently parses as 0.
        let err = Cli::try_parse_from(["duhem", "run", "v.yml", "--seed", "-1"]);
        assert!(err.is_err(), "negative seed should reject");
    }

    #[test]
    fn inputs_file_flag_parses_as_path() {
        let parsed =
            Cli::try_parse_from(["duhem", "run", "v.yml", "--inputs-file", "ci-inputs.yml"])
                .expect("parse");
        match parsed.cmd {
            Some(Cmd::Run { inputs_file, .. }) => {
                assert_eq!(inputs_file, Some(PathBuf::from("ci-inputs.yml")))
            }
            _ => panic!("expected Run"),
        }
    }
}
