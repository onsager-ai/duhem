//! `duhem` — the command-line entry point.
//!
//! Phase-0 skeleton. The free CLI binary (`docs/duhem-spec.md` §13)
//! offers `init`, `validate`, and `run`. `init` (issue #48)
//! scaffolds a runnable Verification Definition skeleton; the `run`
//! subcommand carries the authoring-ergonomics surface
//! (`--filter`, `--evidence-dir`, `--reporter`) per the
//! spec on issue #23. None of the `run` flags are correctness
//! gates — they make iteration on Verification Definitions
//! practical. The headed-browser debug toggle is the `DUHEM_HEADED`
//! env var (spec #151), not a flag.

mod dashboard;
mod environment;
mod filter;
mod init;
mod inputs;
mod reporter;
mod reporter_config;
mod resolve;
mod validate_cmd;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use duhem_actions::RunBrowser;
use duhem_judge::{RunVerdict, VerdictState, aggregate_run_set};
use duhem_runtime::{Engine, RunOutcome, SuiteEnvironment};
use duhem_schema::{Loaded, LoadedLeaf, VerificationDefinition, load as load_definition, validate};

use crate::filter::CliCheckFilter;
use crate::reporter::Reporter;
use crate::reporter_config::PluginRegistry;
use crate::resolve::{render_input_value, resolve_inputs};

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
    /// Parse and structurally validate a Verification Definition, or a
    /// manifest and every leaf it expands to.
    ///
    /// Routes through the same `discover` + `load` pipeline as `run`
    /// (issue #150): a leaf file validates the leaf; a manifest file —
    /// or a directory resolving to one — validates the manifest
    /// structurally and each resolved leaf. Omit the path to
    /// auto-discover a manifest from the cwd and its ancestors, like
    /// `duhem run` (issue #69).
    Validate {
        /// Path to a `.yml` Verification Definition, or a directory /
        /// manifest. Omit to auto-discover a manifest from the current
        /// directory and its ancestors.
        path: Option<PathBuf>,
    },
    /// Execute a Verification Definition end-to-end.
    ///
    /// `--filter`, `--evidence-dir`, `--reporter` are
    /// authoring-ergonomics flags from the spec on issue #23; none
    /// change the verdict on a non-filtered run.
    Run {
        /// Path to a `.yml` Verification Definition, or a directory
        /// containing a manifest. Omit entirely to auto-discover a
        /// manifest from the current directory and its ancestors —
        /// `cd anywhere-in-the-repo && duhem run` (issue #69).
        path: Option<PathBuf>,
        /// Explicit manifest path. Bypasses discovery (no directory
        /// probe, no ancestor walk) and uses the path as-is — the
        /// escape hatch for an out-of-tree manifest like `ops/duhem.yml`.
        /// Mutually exclusive with the positional `path`.
        #[arg(
            short = 'f',
            long = "file",
            value_name = "PATH",
            conflicts_with = "path"
        )]
        file: Option<PathBuf>,
        /// Inputs, repeatable and mixable: `KEY=VALUE` for a single
        /// input, or `@FILE` to load a YAML/JSON mapping (`.yml` /
        /// `.yaml` / `.json`). Tokens are applied left-to-right and the
        /// last mention of a key wins, e.g. `--inputs @base.yml
        /// --inputs k=v --inputs @override.yml`. `@` only loads a file
        /// as a bare leading token; `key=@literal` keeps `@literal` as a
        /// literal value (spec #151).
        #[arg(long = "inputs", value_name = "KEY=VALUE|@FILE")]
        inputs: Vec<String>,
        /// Limit the run to a subset of `(criterion, check)` pairs.
        ///
        /// Grammar: `AC-1` (every check under `AC-1`),
        /// `AC-1::AC-1.2` (one pair), `AC-*::AC-*.1` (globbed). Repeat
        /// the flag to OR patterns: `--filter AC-1 --filter AC-2`.
        #[arg(long = "filter", value_name = "PATTERN")]
        filter: Vec<String>,
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
        /// Select a named environment from the manifest's
        /// `environments:` block (spec #68). The selected environment's
        /// keys feed input resolution (below `--inputs`, above the VD
        /// `default:`) and its
        /// string-valued keys are reachable via `$env.<key>`. When the
        /// manifest declares exactly one environment it auto-selects;
        /// with two or more, this flag is required. Inert on a
        /// single-leaf run (no manifest).
        #[arg(long = "environment", value_name = "NAME")]
        environment: Option<String>,
        /// Parse + validate the definition, resolve the filter, print
        /// the `(criterion::check)` pairs that *would* run, and exit
        /// 0 without launching the browser or writing evidence. Use
        /// when authoring a Verification Definition to confirm a
        /// `--filter` resolves to the pairs you expect (spec on #33).
        #[arg(long = "dry-run", default_value_t = false)]
        dry_run: bool,
        /// Skip `environment.up:` and readiness probing. Use when the
        /// operator brought the SUT up out-of-band. Teardown still
        /// runs unless `--keep-env` is also passed. Has no effect on
        /// VDs without an `environment:` block. Spec on issue #50.
        #[arg(long = "no-env-up", default_value_t = false)]
        no_env_up: bool,
        /// Skip `environment.down:`. Use when an author wants the
        /// SUT to outlive the run for triage. Has no effect on VDs
        /// without an `environment:` block. Spec on issue #50.
        #[arg(long = "keep-env", default_value_t = false)]
        keep_env: bool,
    },
    /// Browse run evidence in a read-only web dashboard.
    ///
    /// Shells out to the separate `duhem-dashboard` binary (specs
    /// #53 / #87); `dashboard.rs` owns binary resolution and the
    /// serve/export surface.
    Dashboard(dashboard::DashboardOpts),
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
        }) => init::run_init(path, &pattern, name, force),
        Some(Cmd::Validate { path }) => match validate_cmd::run_validate(path.as_deref()) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        },
        Some(Cmd::Dashboard(opts)) => dashboard::run(&opts.into()),
        Some(Cmd::Run {
            path,
            file,
            inputs,
            filter,
            evidence_dir,
            reporter,
            environment,
            dry_run,
            no_env_up,
            keep_env,
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
                file,
                inputs,
                filter,
                evidence_dir,
                reporter: resolved_reporter,
                environment,
                dry_run,
                no_env_up,
                keep_env,
            }))
        }
    }
}

/// Resolved `duhem run` arguments. Kept as a struct so the dispatch
/// function's signature doesn't grow unbounded as new flags land.
struct RunArgs {
    path: Option<PathBuf>,
    file: Option<PathBuf>,
    inputs: Vec<String>,
    filter: Vec<String>,
    evidence_dir: Option<PathBuf>,
    reporter: Reporter,
    environment: Option<String>,
    dry_run: bool,
    no_env_up: bool,
    keep_env: bool,
}

async fn run_command(args: RunArgs) -> ExitCode {
    let RunArgs {
        path,
        file,
        inputs: raw_inputs,
        filter: raw_filter,
        evidence_dir,
        reporter,
        environment: requested_environment,
        dry_run,
        no_env_up,
        keep_env,
    } = args;

    // The headed-browser debug toggle is the `DUHEM_HEADED` env var
    // (spec #151): truthy `1` / `true` (case-insensitive) launches a
    // visible window; anything else (or unset) stays headless. It has no
    // effect on `api/*` / page-free runs that never launch a browser.
    let headed = env_headed();

    // Resolve which manifest/leaf to load (issue #69). `-f`/`--file` is
    // the explicit override — used as-is, no discovery. Otherwise
    // `discover` resolves the positional `path` (a file verbatim, a
    // directory probed for a manifest) or, when no path is given, walks
    // the cwd and its ancestors so `cd anywhere-in-the-repo && duhem
    // run` finds the repo-root manifest (capped at a `.git` boundary).
    let target = match file {
        Some(f) => f,
        None => {
            let cwd = match std::env::current_dir() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("cannot determine current directory: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match duhem_schema::discover(path.as_deref(), &cwd) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[schema v{}] {e}", duhem_schema::SCHEMA_VERSION);
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    // Polymorphic load: directory → first manifest candidate; manifest →
    // expand leaves; leaf → single Verification Definition (today's
    // behavior). Spec on issue #49. The loader annotates YAML / shape
    // failures with the offending path; we prefix the schema version
    // so authors see at a glance which schema the loader parsed
    // against (spec on #51).
    let loaded = match load_definition(&target) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[schema v{}] {e}", duhem_schema::SCHEMA_VERSION);
            return ExitCode::FAILURE;
        }
    };

    // Fold the `--inputs` tokens (`KEY=VALUE` + `@file`, last-wins) into
    // one merged map before resolving, so any `@file` load (and a
    // missing/malformed file) fails fast — before a browser launch —
    // and the file values participate in the same required/unknown/typed
    // checks as `KEY=VALUE` tokens. Inputs apply to every leaf the
    // manifest expands to — per the issue, the manifest does not remap
    // inputs per leaf in v1.
    let merged_inputs = match inputs::merge_inputs(&raw_inputs) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
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
        Manifest {
            warnings: Vec<String>,
            /// Shared environment provisioned once for the whole suite
            /// (spec #131), with the manifest's parent dir for anchoring
            /// its `up:` / `down:` scripts.
            environment: Option<duhem_schema::Environment>,
            manifest_dir: Option<PathBuf>,
            /// Suite-wide `defaults:` block (spec #66) applied to every
            /// leaf's engine: per-step `within:` fallback, inconclusive
            /// policy, retry posture. `None` on a defaults-less manifest.
            defaults: Option<duhem_schema::ManifestDefaults>,
        },
    }
    // Named-environment selection (spec #68). On a manifest we pick the
    // run's environment from the manifest's `environments:` block and
    // the `--environment` flag; the projection feeds both input
    // resolution and the `$env.<key>` whitelist. On a single leaf there
    // is no manifest, so nothing is selected; a `--environment` passed
    // there is inert (warned below).
    let mut selected_env: Option<environment::SelectedEnvironment> = None;
    let (leaves, scope): (Vec<LoadedLeaf>, Scope) = match loaded {
        Loaded::Leaf { path, definition } => {
            if requested_environment.is_some() {
                eprintln!(
                    "warning: --environment has no effect on a single-leaf run (no manifest with an `environments:` block)"
                );
            }
            (vec![LoadedLeaf { path, definition }], Scope::SingleLeaf)
        }
        Loaded::Manifest {
            manifest_path,
            manifest,
            leaves,
            warnings,
        } => {
            match environment::select_environment(
                &manifest.environments,
                requested_environment.as_deref(),
            ) {
                Ok(sel) => {
                    if let Some(s) = &sel {
                        eprintln!("environment: {}", s.name);
                    }
                    selected_env = sel;
                }
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::FAILURE;
                }
            }
            let manifest_dir = manifest_path.parent().map(Path::to_path_buf);
            (
                leaves,
                Scope::Manifest {
                    warnings,
                    environment: manifest.environment,
                    manifest_dir,
                    defaults: manifest.defaults,
                },
            )
        }
    };
    // Input-resolution view of the selected environment (precedence
    // layer 3); empty when nothing is selected so the resolution chain
    // is unchanged on environment-free runs.
    let env_inputs: BTreeMap<String, serde_json::Value> = selected_env
        .as_ref()
        .map(|s| s.inputs.clone())
        .unwrap_or_default();
    // `$env.<key>` whitelist seed (string-valued keys only).
    let env_whitelist: BTreeMap<String, String> = selected_env
        .as_ref()
        .map(|s| s.env.clone())
        .unwrap_or_default();
    if let Scope::Manifest { warnings, .. } = &scope {
        for w in warnings {
            eprintln!("warning: {w}");
        }
    }
    let is_manifest = matches!(scope, Scope::Manifest { .. });
    // Suite-wide `defaults:` (spec #66) applied to every leaf's engine.
    // A single-leaf run has no manifest, so defaults are inert there —
    // Pattern A authors pay no cost.
    let manifest_defaults: Option<duhem_schema::ManifestDefaults> = match &scope {
        Scope::Manifest { defaults, .. } => defaults.clone(),
        Scope::SingleLeaf => None,
    };

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
        let inputs = match resolve_inputs(
            &merged_inputs,
            &env_inputs,
            &leaf.definition.inputs,
            &leaf.definition.inherits,
        ) {
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
        // RESOLVED INPUTS (spec #155): the post-precedence input map
        // (`--inputs` last-wins > selected environment > VD `default:`),
        // one `name = value` per line so a black-box VD can assert the
        // winning value directly off stdout — the value-level assertion
        // that was only reachable indirectly before (via type levers).
        // Qualified by verification name on manifest runs, mirroring the
        // `WOULD RUN` lines. Values render deterministically (strings
        // bare, other types as compact JSON of the coerced value).
        for (name, _path, _def, inputs) in &resolved {
            let leaf_filter = check_filter.as_ref().and_then(|f| f.for_verification(name));
            if check_filter.is_some() && leaf_filter.is_none() {
                continue;
            }
            if inputs.is_empty() {
                let line = if is_manifest {
                    format!("RESOLVED INPUT: {name}:: (none)")
                } else {
                    "RESOLVED INPUT: (none)".to_string()
                };
                if let Err(e) = writeln!(stdout, "{line}") {
                    eprintln!("dry-run: {e}");
                    return ExitCode::FAILURE;
                }
                continue;
            }
            for (key, value) in inputs {
                let rendered = render_input_value(value);
                let line = if is_manifest {
                    format!("RESOLVED INPUT: {name}::{key} = {rendered}")
                } else {
                    format!("RESOLVED INPUT: {key} = {rendered}")
                };
                if let Err(e) = writeln!(stdout, "{line}") {
                    eprintln!("dry-run: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        if let Err(e) = stdout.flush() {
            eprintln!("dry-run: {e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    // Manifest-level shared environment (spec #131): provision the whole
    // suite's stack once, here, instead of each leaf standing up its own.
    // While it's up, leaves run with per-leaf provisioning suppressed
    // (`suite_managed`) and target the shared stack.
    let mut suite_env: Option<SuiteEnvironment> = None;
    if let Scope::Manifest {
        environment: Some(env),
        manifest_dir,
        ..
    } = &scope
    {
        let suite_dir = evidence_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from(".duhem/runs"))
            .join("_suite");
        match SuiteEnvironment::provision(env, manifest_dir.as_deref(), &suite_dir, no_env_up).await
        {
            Ok(session) => {
                if let Some(cause) = session.aborted_cause() {
                    eprintln!("suite environment did not come up: {cause:?}");
                    let _ = session.tear_down(keep_env).await;
                    return ExitCode::FAILURE;
                }
                suite_env = Some(session);
            }
            Err(e) => {
                eprintln!("suite environment: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    let suite_managed = suite_env.is_some();

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

        // Only launch the Playwright sidecar when the leaf actually
        // drives a page. A pure `api/*` + `db/*` + `cli/*` verification
        // needs no browser, so we skip the launch (and its Chromium
        // dependency + startup cost) entirely. `uses_requires_page` is
        // the same classifier the engine uses to gate the per-check
        // browser, so this never starves a UI step of a page.
        let needs_browser = def
            .criteria
            .iter()
            .flat_map(|c| &c.checks)
            .flat_map(|ch| &ch.steps)
            .any(|s| duhem_actions::uses_requires_page(&s.uses));

        // One browser per leaf when needed. Phase-0 leaves run serially
        // (#49) and `RunBrowser` is non-`Clone`, so a fresh launch per
        // leaf is the cleanest model.
        let browser = if needs_browser {
            match RunBrowser::launch(headed).await {
                Ok(b) => Some(b),
                Err(e) => {
                    eprintln!("browser: {e}");
                    if let Some(s) = suite_env.take() {
                        let _ = s.tear_down(keep_env).await;
                    }
                    return ExitCode::FAILURE;
                }
            }
        } else {
            None
        };

        // Under a manifest's shared environment, the leaf must not stand
        // up or tear down its own — the suite owns the stack.
        let mut engine = Engine::new()
            .with_definition_path(leaf_path.display().to_string())
            .skip_env_up(no_env_up || suite_managed)
            .keep_env(keep_env || suite_managed)
            .with_env(env_whitelist.clone())
            .with_inherited(def.inherits.clone());
        if let Some(d) = manifest_defaults.as_ref() {
            engine = engine.with_defaults(d);
        }
        if let Some(b) = browser {
            engine = engine.with_browser(b);
        }
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
        let outcome = match engine.run_with_metadata(&def, inputs).await {
            Ok(o) => o,
            Err(e) => {
                eprintln!("engine ({}): {e}", leaf_path.display());
                if let Some(s) = suite_env.take() {
                    let _ = s.tear_down(keep_env).await;
                }
                return ExitCode::FAILURE;
            }
        };
        leaf_outcomes.push((name, outcome));
    }

    // Tear the shared suite stack down once, after the last leaf
    // (best-effort; `--keep-env` leaves it up for triage).
    if let Some(session) = suite_env.take()
        && let Err(e) = session.tear_down(keep_env).await
    {
        eprintln!("suite teardown: {e}");
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

/// Read the `DUHEM_HEADED` env var and decide whether to launch a
/// visible browser (spec #151). Truthy is `1` / `true`, case-
/// insensitive and whitespace-trimmed; everything else (including an
/// unset var) is headless — the default.
fn env_headed() -> bool {
    std::env::var("DUHEM_HEADED")
        .ok()
        .is_some_and(|v| parse_truthy(&v))
}

/// The truthiness rule for `DUHEM_HEADED`. Pure (no env access) so it
/// is unit-testable without mutating process-global state.
fn parse_truthy(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true")
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use duhem_schema::InputDecl;

    /// Spec on #151: the headed-browser toggle moved from `--headed` to
    /// the `DUHEM_HEADED` env var. `--headed` is gone from the CLI
    /// surface (clap rejects it — see `run_flags.rs`); here we pin the
    /// truthiness rule `env_headed` reads, without mutating
    /// process-global env state.
    #[test]
    fn parse_truthy_accepts_1_and_true_case_insensitively() {
        for truthy in ["1", "true", "TRUE", "True", " true ", "tRuE"] {
            assert!(parse_truthy(truthy), "`{truthy}` should be truthy");
        }
        for falsy in ["", "0", "false", "no", "yes", "2", "on", "headed"] {
            assert!(!parse_truthy(falsy), "`{falsy}` should be falsy");
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

    /// Build a merged `--inputs` map from `KEY=VALUE` / `@file` tokens,
    /// the same fold the CLI does (`inputs::merge_inputs`).
    fn merged(tokens: &[&str]) -> BTreeMap<String, inputs::InputValue> {
        inputs::merge_inputs(&raw(tokens)).expect("merge tokens")
    }

    /// A merged map of already-typed values, standing in for what an
    /// `--inputs @file` token contributes (each key an `InputValue::Typed`).
    fn typed(pairs: &[(&str, serde_json::Value)]) -> BTreeMap<String, inputs::InputValue> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), inputs::InputValue::Typed(v.clone())))
            .collect()
    }

    /// Test-only shorthand: resolve `KEY=VALUE` tokens with no `@file`,
    /// no environment, no inherited names. The `@file` / environment /
    /// inherited code paths are exercised separately below.
    fn resolve(
        cli: &[String],
        decls: &BTreeMap<String, InputDecl>,
    ) -> Result<BTreeMap<String, serde_json::Value>, String> {
        let empty = BTreeMap::new();
        let merged = inputs::merge_inputs(cli).expect("merge tokens");
        resolve_inputs(&merged, &empty, decls, &[])
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

    // ---- #151: `--inputs` merged-token resolution (`@file` typed
    // values + `KEY=VALUE` raw values; last-wins handled in
    // `inputs::merge_inputs`) ----

    #[test]
    fn at_file_typed_value_supplies_input_when_no_flag() {
        // An `@file` contributes already-typed JSON; it resolves when no
        // `KEY=VALUE` token mentions the key.
        let d = decls("  count: { type: integer }");
        let out = resolve_inputs(
            &typed(&[("count", serde_json::json!(7))]),
            &BTreeMap::new(),
            &d,
            &[],
        )
        .unwrap();
        assert_eq!(out["count"], serde_json::json!(7));
    }

    #[test]
    fn at_file_typed_value_validates_against_declared_type() {
        // A file value whose JSON shape doesn't match the declared
        // `InputType` is a real authoring error — surface it as a
        // CLI-side failure, not as a confusing runtime
        // `Inconclusive(TypeMismatch)` later.
        let d = decls("  count: { type: integer }");
        let err = resolve_inputs(
            &typed(&[("count", serde_json::json!("not a number"))]),
            &BTreeMap::new(),
            &d,
            &[],
        )
        .unwrap_err();
        assert!(
            err.contains("count") && err.contains("integer"),
            "got: {err}"
        );
    }

    #[test]
    fn unknown_input_from_at_file_is_an_error() {
        let d = decls("  count: { type: integer }");
        let err = resolve_inputs(
            &typed(&[
                ("bogus", serde_json::json!(1)),
                ("count", serde_json::json!(1)),
            ]),
            &BTreeMap::new(),
            &d,
            &[],
        )
        .unwrap_err();
        assert!(
            err.contains("unknown input") && err.contains("bogus"),
            "got: {err}"
        );
    }

    #[test]
    fn flag_value_overrides_default() {
        // Resolution order: `--inputs` (raw or typed) > declared default
        // > error.
        let d = decls("  name: { type: string, default: ws-default }");
        let out = resolve(&raw(&["name=from-flag"]), &d).unwrap();
        assert_eq!(out["name"], serde_json::json!("from-flag"));
    }

    #[test]
    fn at_file_value_overrides_default() {
        let d = decls("  name: { type: string, default: ws-default }");
        let out = resolve_inputs(
            &typed(&[("name", serde_json::json!("from-file"))]),
            &BTreeMap::new(),
            &d,
            &[],
        )
        .unwrap();
        assert_eq!(out["name"], serde_json::json!("from-file"));
    }

    // ---- #68: selected-environment input resolution (precedence:
    // --inputs (last-wins) > selected env > VD default) ----

    #[test]
    fn env_supplies_input_when_nothing_higher_does() {
        let d = decls("  base_url: { type: string }");
        let mut env = BTreeMap::new();
        env.insert("base_url".into(), serde_json::json!("https://staging"));
        let out = resolve_inputs(&merged(&[]), &env, &d, &[]).unwrap();
        assert_eq!(out["base_url"], serde_json::json!("https://staging"));
    }

    #[test]
    fn explicit_inputs_override_env() {
        let d = decls("  base_url: { type: string }");
        let mut env = BTreeMap::new();
        env.insert("base_url".into(), serde_json::json!("https://staging"));
        let out = resolve_inputs(&merged(&["base_url=from-flag"]), &env, &d, &[]).unwrap();
        assert_eq!(out["base_url"], serde_json::json!("from-flag"));
    }

    #[test]
    fn at_file_and_flag_both_override_env() {
        let d = decls("  base_url: { type: string }");
        let mut env = BTreeMap::new();
        env.insert("base_url".into(), serde_json::json!("from-env"));
        // an `@file` typed value beats env
        let out = resolve_inputs(
            &typed(&[("base_url", serde_json::json!("from-file"))]),
            &env,
            &d,
            &[],
        )
        .unwrap();
        assert_eq!(out["base_url"], serde_json::json!("from-file"));
        // a `KEY=VALUE` raw value beats env
        let out = resolve_inputs(&merged(&["base_url=from-flag"]), &env, &d, &[]).unwrap();
        assert_eq!(out["base_url"], serde_json::json!("from-flag"));
    }

    #[test]
    fn vd_default_is_the_floor_below_env() {
        let d = decls("  base_url: { type: string, default: from-default }");
        // env supplies → env wins over the default
        let mut env = BTreeMap::new();
        env.insert("base_url".into(), serde_json::json!("from-env"));
        let out = resolve_inputs(&merged(&[]), &env, &d, &[]).unwrap();
        assert_eq!(out["base_url"], serde_json::json!("from-env"));
        // env absent → default is the floor
        let out = resolve_inputs(&merged(&[]), &BTreeMap::new(), &d, &[]).unwrap();
        assert_eq!(out["base_url"], serde_json::json!("from-default"));
    }

    #[test]
    fn env_key_with_no_declared_input_is_ignored() {
        // An environment may carry keys consumed only via `$env.<key>`;
        // such a key matching no declared input must not error.
        let d = decls("  base_url: { type: string }");
        let mut env = BTreeMap::new();
        env.insert("base_url".into(), serde_json::json!("https://staging"));
        env.insert("db_url".into(), serde_json::json!("postgres://x"));
        let out = resolve_inputs(&merged(&[]), &env, &d, &[]).unwrap();
        assert_eq!(out["base_url"], serde_json::json!("https://staging"));
        assert!(!out.contains_key("db_url"));
    }

    #[test]
    fn env_value_type_mismatch_is_an_error() {
        let d = decls("  count: { type: integer }");
        let mut env = BTreeMap::new();
        env.insert("count".into(), serde_json::json!("not a number"));
        let err = resolve_inputs(&merged(&[]), &env, &d, &[]).unwrap_err();
        assert!(
            err.contains("count") && err.contains("environment"),
            "got: {err}"
        );
    }

    #[test]
    fn environment_flag_parses() {
        let parsed =
            Cli::try_parse_from(["duhem", "run", "v.yml", "--environment", "prod"]).expect("parse");
        match parsed.cmd {
            Some(Cmd::Run { environment, .. }) => {
                assert_eq!(environment, Some("prod".to_string()));
            }
            _ => panic!("expected Run"),
        }
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

    /// Spec on #151: `--seed`, `--headed`, and `--inputs-file` were
    /// pruned from `duhem run`. clap must now reject each as an unknown
    /// argument (the breaking CLI surface change). The black-box
    /// stderr/exit-code shape is pinned in `run_flags.rs`; here we pin
    /// the parser rejection.
    #[test]
    fn removed_flags_are_rejected_by_clap() {
        assert!(
            Cli::try_parse_from(["duhem", "run", "v.yml", "--seed", "42"]).is_err(),
            "--seed should be unknown"
        );
        assert!(
            Cli::try_parse_from(["duhem", "run", "v.yml", "--headed"]).is_err(),
            "--headed should be unknown"
        );
        assert!(
            Cli::try_parse_from(["duhem", "run", "v.yml", "--inputs-file", "x.yml"]).is_err(),
            "--inputs-file should be unknown"
        );
    }

    #[test]
    fn env_lifecycle_flags_parse_and_default_off() {
        // Spec on #50: `--no-env-up` and `--keep-env` are independent
        // escape hatches; both default off so the runtime manages the
        // full lifecycle when `environment:` is present.
        let default = Cli::try_parse_from(["duhem", "run", "v.yml"]).expect("parse");
        match default.cmd {
            Some(Cmd::Run {
                no_env_up,
                keep_env,
                ..
            }) => {
                assert!(!no_env_up, "default off");
                assert!(!keep_env, "default off");
            }
            _ => panic!("expected Run"),
        }
        let opted = Cli::try_parse_from(["duhem", "run", "v.yml", "--no-env-up", "--keep-env"])
            .expect("parse");
        match opted.cmd {
            Some(Cmd::Run {
                no_env_up,
                keep_env,
                ..
            }) => {
                assert!(no_env_up);
                assert!(keep_env);
            }
            _ => panic!("expected Run"),
        }
    }

    // ---- #69: manifest discovery (`-f`/`--file`, optional path) ----

    #[test]
    fn file_override_parses_and_path_is_optional() {
        // `-f` supplies the manifest path and the positional `path` may
        // be omitted entirely (discovery handles the bare invocation).
        let with_file =
            Cli::try_parse_from(["duhem", "run", "-f", "ops/duhem.yml"]).expect("parse");
        match with_file.cmd {
            Some(Cmd::Run { path, file, .. }) => {
                assert!(path.is_none(), "positional path absent");
                assert_eq!(file, Some(PathBuf::from("ops/duhem.yml")));
            }
            _ => panic!("expected Run"),
        }
        // Long form `--file` is the documented alias for `-f`.
        let long = Cli::try_parse_from(["duhem", "run", "--file", "ops/duhem.yml"]).expect("parse");
        match long.cmd {
            Some(Cmd::Run { file, .. }) => assert_eq!(file, Some(PathBuf::from("ops/duhem.yml"))),
            _ => panic!("expected Run"),
        }
        // Bare `duhem run` (no path, no `-f`) parses — discovery runs.
        let bare = Cli::try_parse_from(["duhem", "run"]).expect("parse");
        match bare.cmd {
            Some(Cmd::Run { path, file, .. }) => {
                assert!(path.is_none());
                assert!(file.is_none());
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn file_flag_conflicts_with_positional_path() {
        // `-f` and a positional `path` are mutually exclusive at the
        // clap level (issue #69 §Design).
        let err = Cli::try_parse_from(["duhem", "run", "v.yml", "-f", "ops/duhem.yml"]);
        assert!(err.is_err(), "positional path + -f must conflict");
    }
}
