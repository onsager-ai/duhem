//! `duhem run` — dispatch and execution (split from `main.rs` for
//! the file-token budget; the clap surface stays in `main.rs`).

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use duhem_actions::RunBrowser;
use duhem_evidence::{SqliteStore, Store};
use duhem_judge::{RunVerdict, VerdictState, aggregate_run_set};
use duhem_runtime::{Engine, RunOutcome, SuiteEnvironment};
use duhem_schema::{
    Loaded, LoadedLeaf, VerificationDefinition, load as load_definition,
    validate_with_contract_outputs,
};

use crate::environment;
use crate::filter::CliCheckFilter;
use crate::inputs;
use crate::reporter::{self, Reporter};
use crate::resolve::{render_input_value, resolve_inputs};

/// Resolved `duhem run` arguments. Kept as a struct so the dispatch
/// function's signature doesn't grow unbounded as new flags land.
pub struct RunArgs {
    pub path: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub inputs: Vec<String>,
    pub filter: Vec<String>,
    pub db: Option<PathBuf>,
    pub run_id: Option<String>,
    pub reporter: Reporter,
    pub environment: Option<String>,
    pub dry_run: bool,
    pub no_env_up: bool,
    pub keep_env: bool,
    /// Live per-criterion progress on stderr (#299): `Some(true)` /
    /// `Some(false)` from `--live` / `--no-live`; `None` = auto
    /// (on iff stderr is a TTY).
    pub live: Option<bool>,
    /// Open the dashboard's live run page in a browser (#305),
    /// once per invocation. Warns and continues when no dashboard
    /// base resolves.
    pub watch: bool,
    pub capture: duhem_runtime::CapturePolicy,
    pub capture_video: bool,
}

pub async fn run_command(args: RunArgs) -> ExitCode {
    let RunArgs {
        path,
        file,
        inputs: raw_inputs,
        filter: raw_filter,
        db,
        run_id: pinned_run_id,
        reporter,
        environment: requested_environment,
        dry_run,
        no_env_up,
        keep_env,
        live,
        watch,
        capture,
        capture_video,
    } = args;

    // Live progress posture (#299), resolved once: forced by
    // `--live`/`--no-live`, else on iff stderr is a terminal (a piped
    // or CI stderr stays clean without asking).
    let live_progress = live.unwrap_or_else(|| std::io::stderr().is_terminal());
    // Presentation + heartbeat cadence (#305): in-place single-line
    // rewriting on a real terminal, plain append lines into a capture.
    let render_cfg = crate::live_progress::RenderConfig::detect();

    // Recording is enabled at context-open time, so it's a run-level
    // decision (#215). Only record when the user asked *and* captures
    // aren't disabled outright — otherwise every context would record a
    // video that `--capture off` immediately discards.
    let record_video = capture_video && !matches!(capture, duhem_runtime::CapturePolicy::Off);

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
    // One-shot dispatch enum; the size skew vs `SingleLeaf` is
    // irrelevant for a single stack value per invocation.
    #[allow(clippy::large_enum_variant)]
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
            /// Suite-wide declared target (#191); a leaf's own
            /// `project:` wins over it.
            project: Option<duhem_schema::ProjectDecl>,
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
                    "warning: --environment has no effect when running a single verification (no manifest with an `environments:` block)"
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
                    project: manifest.project,
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
    // Suite-wide declared target (#191); leaf `project:` wins below.
    let manifest_project: Option<duhem_schema::ProjectDecl> = match &scope {
        Scope::Manifest { project, .. } => project.clone(),
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
        if let Err(errs) = validate_with_contract_outputs(&leaf.definition, &|u| {
            crate::contract_check::contract_outputs(u)
        }) {
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

    // Open the evidence store (#189): the explicit `--db` path, else
    // the working copy's project DB under the duhem state dir. One
    // store per invocation; every leaf run (and the suite-environment
    // run) lands in it. Opened only after `--dry-run` returned, so a
    // dry run writes nothing.
    let db_path = match &db {
        Some(p) => p.clone(),
        None => {
            let cwd = match std::env::current_dir() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("cannot determine current directory: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match duhem_evidence::project_db_path(&cwd) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("resolve store: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    };
    let store: Arc<dyn Store> = match SqliteStore::open(&db_path).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("open store {}: {e}", db_path.display());
            return ExitCode::FAILURE;
        }
    };

    // Live on-ramp (#298): if a dashboard is serving this store (or
    // the operator exported DUHEM_DASHBOARD_URL), each leaf's live
    // deep link is printed before its run starts. Resolved once per
    // invocation; None costs nothing downstream.
    let dashboard_base = crate::live_link::resolve_dashboard_base(&db_path);
    // #305: `--watch` needs somewhere to point the browser; without a
    // base it degrades to a warning, never a failed run.
    if watch && dashboard_base.is_none() {
        eprintln!(
            "warning: --watch: no dashboard is serving this store — start `duhem dashboard` or set DUHEM_DASHBOARD_URL"
        );
    }

    // Manifest-level shared environment (spec #131): provision the whole
    // suite's stack once, here, instead of each leaf standing up its own.
    // While it's up, leaves run with per-leaf provisioning suppressed
    // (`suite_managed`) and target the shared stack.
    let mut suite_env: Option<SuiteEnvironment> = None;
    // #305 A: while the suite environment provisions (and later tears
    // down), its evidence tee narrates on stderr — the often-slowest
    // minute of a manifest run, like leaf-level env blocks already
    // do. Rendering is driven inline (select + drain) rather than by
    // a spawned task so `env:` lines order deterministically against
    // run_cmd's own stderr lines (headers, live links).
    let mut suite_progress: Option<tokio::sync::mpsc::UnboundedReceiver<duhem_evidence::Event>> =
        None;
    if let Scope::Manifest {
        environment: Some(env),
        manifest_dir,
        ..
    } = &scope
    {
        let progress = if live_progress {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            suite_progress = Some(rx);
            Some(tx)
        } else {
            None
        };
        let provision = SuiteEnvironment::provision(
            env,
            manifest_dir.as_deref(),
            store.clone(),
            no_env_up,
            progress,
        );
        let outcome = match suite_progress.as_mut() {
            Some(rx) => {
                let mut fut = std::pin::pin!(provision);
                let res = loop {
                    tokio::select! {
                        biased;
                        Some(evt) = rx.recv() => {
                            crate::live_progress::narrate_env_event_to_stderr(&evt);
                        }
                        res = &mut fut => break res,
                    }
                };
                // Events sent during the future's final poll are still
                // queued; drain them before anything else prints.
                while let Ok(evt) = rx.try_recv() {
                    crate::live_progress::narrate_env_event_to_stderr(&evt);
                }
                res
            }
            None => provision.await,
        };
        match outcome {
            Ok(session) => {
                if let Some(cause) = session.aborted_cause() {
                    eprintln!("suite environment did not come up: {cause:?}");
                    let _ = tear_down_suite(session, keep_env, &mut suite_progress).await;
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

    if pinned_run_id.is_some() && (is_manifest || resolved.len() > 1) {
        eprintln!(
            "--run-id applies to a single-verification run; a manifest expands to several verifications"
        );
        if let Some(s) = suite_env.take() {
            let _ = tear_down_suite(s, keep_env, &mut suite_progress).await;
        }
        return ExitCode::FAILURE;
    }

    let mut leaf_outcomes: Vec<(String, RunOutcome)> = Vec::with_capacity(resolved.len());
    let total = resolved.len();
    let mut watch_opened = false;
    for (idx, (name, leaf_path, def, inputs)) in resolved.into_iter().enumerate() {
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

        // #305 B: on a live manifest run, a header separates each
        // verification's narration. The name is the same path-derived
        // slug the evidence namespace, set summary, and dashboard
        // already use.
        if live_progress && is_manifest {
            eprintln!("── {name} ({}/{total}) ──", idx + 1);
        }

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
                Ok(b) => Some(b.with_video(record_video)),
                Err(e) => {
                    eprintln!("browser: {e}");
                    if let Some(s) = suite_env.take() {
                        let _ = tear_down_suite(s, keep_env, &mut suite_progress).await;
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
            .with_inherited(def.inherits.clone())
            .with_capture(capture);
        if let Some(d) = manifest_defaults.as_ref() {
            engine = engine.with_defaults(d);
        }
        if let Some(b) = browser {
            engine = engine.with_browser(b);
        }
        engine = engine.with_store(store.clone());
        // Run-id pinning: an operator `--run-id` wins; otherwise, when
        // a live link is printable, mint the id here (the same ULID
        // mint the engine would run) so the URL exists before the run
        // does. With no dashboard in sight the engine mints as before.
        let run_id: Option<String> = match pinned_run_id.as_deref() {
            Some(id) => Some(id.to_string()),
            None if dashboard_base.is_some() => Some(duhem_evidence::new_run_id()),
            None => None,
        };
        if let Some(id) = run_id.as_deref() {
            engine = engine.with_run_id(id);
        }
        // stderr, before the (blocking) run: stdout stays byte-stable
        // for machine reporters, and the operator can click in while
        // the run is still executing — the run page streams live.
        if let (Some(base), Some(id)) = (dashboard_base.as_deref(), run_id.as_deref()) {
            let url = crate::live_link::run_page_url(base, id);
            eprintln!("live: {url}");
            // #305 D: `--watch` opens the page once per invocation —
            // the first verification's page; the dashboard links the
            // rest of the suite from there.
            if watch && !watch_opened {
                crate::live_link::open_in_browser(&url);
                watch_opened = true;
            }
        }
        // Target identity (#191): the declared `project:` (leaf wins
        // over manifest) enters the resolution ladder; the VD's
        // directory anchors the verifier side.
        {
            let declared = def.project.as_ref().or(manifest_project.as_ref());
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let vd_dir = leaf_path.parent().map(Path::to_path_buf);
            engine = engine.with_scope(duhem_runtime::resolve_scope(
                declared,
                &cwd,
                vd_dir.as_deref(),
            ));
        }
        if let Some(f) = leaf_filter {
            engine = engine.with_filter(f);
        }
        // Live terminal progress (#299): subscribe a renderer to the
        // engine's progress sink. It folds the same evidence events
        // the store persists onto stderr and returns at
        // `run_finished`; on the engine-error path (no `run_finished`
        // arrives) it is aborted rather than awaited.
        let renderer = if live_progress {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let plan = crate::live_progress::Plan::from_def(&def);
            engine = engine.with_progress(tx);
            Some(tokio::spawn(crate::live_progress::render_to_stderr(
                rx, plan, render_cfg,
            )))
        } else {
            None
        };
        let outcome = match engine.run_with_metadata(&def, inputs).await {
            Ok(o) => o,
            Err(e) => {
                if let Some(r) = renderer {
                    r.abort();
                }
                eprintln!("engine ({}): {e}", leaf_path.display());
                if let Some(s) = suite_env.take() {
                    let _ = tear_down_suite(s, keep_env, &mut suite_progress).await;
                }
                return ExitCode::FAILURE;
            }
        };
        if let Some(r) = renderer {
            // Returns as soon as the renderer consumed `run_finished`
            // (already sent — the run is over), so no reporter output
            // interleaves with a late progress line.
            let _ = r.await;
        }
        leaf_outcomes.push((name, outcome));
    }

    // Tear the shared suite stack down once, after the last leaf
    // (best-effort; `--keep-env` leaves it up for triage).
    if let Some(session) = suite_env.take()
        && let Err(e) = tear_down_suite(session, keep_env, &mut suite_progress).await
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
        if let Err(e) = reporter::render(&reporter, &mut stdout, outcome, &db_path) {
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
    if let Err(e) = reporter::render_set(
        &reporter,
        &mut stdout,
        &leaf_outcomes,
        &set_verdict,
        &db_path,
    ) {
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

/// Tear the suite environment down, narrating its `env:` events live
/// when a suite progress receiver exists (#305 A). Consumes the
/// receiver: teardown drops the suite writer, closing the channel,
/// so the final drain is bounded.
async fn tear_down_suite(
    session: SuiteEnvironment,
    keep_env: bool,
    progress: &mut Option<tokio::sync::mpsc::UnboundedReceiver<duhem_evidence::Event>>,
) -> Result<(), duhem_runtime::EngineError> {
    let res = match progress.as_mut() {
        Some(rx) => {
            let mut fut = std::pin::pin!(session.tear_down(keep_env));
            let res = loop {
                tokio::select! {
                    biased;
                    Some(evt) = rx.recv() => {
                        crate::live_progress::narrate_env_event_to_stderr(&evt);
                    }
                    res = &mut fut => break res,
                }
            };
            while let Ok(evt) = rx.try_recv() {
                crate::live_progress::narrate_env_event_to_stderr(&evt);
            }
            res
        }
        None => session.tear_down(keep_env).await,
    };
    *progress = None;
    res
}

/// Read the `DUHEM_HEADED` env var and decide whether to launch a
/// visible browser (spec #151). Truthy is `1` / `true`, case-
/// insensitive and whitespace-trimmed; everything else (including an
/// unset var) is headless — the default.
pub(crate) fn env_headed() -> bool {
    std::env::var("DUHEM_HEADED")
        .ok()
        .is_some_and(|v| parse_truthy(&v))
}

/// The truthiness rule for `DUHEM_HEADED`. Pure (no env access) so it
/// is unit-testable without mutating process-global state.
pub(crate) fn parse_truthy(value: &str) -> bool {
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
pub(crate) fn leaf_name(path: &std::path::Path) -> String {
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
