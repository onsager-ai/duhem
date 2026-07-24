//! `duhem` — the command-line entry point.
//!
//! Phase-0 skeleton. The free CLI binary (`docs/duhem-spec.md` §13)
//! offers `init`, `validate`, `run`, `export`, and `dashboard`.
//! `init` (issue #48) scaffolds a runnable Verification Definition
//! skeleton; the `run` subcommand carries the authoring-ergonomics
//! surface (`--filter`, `--db`, `--reporter`) per the spec on issue
//! #23 (+ #189 for the store). None of the `run` flags are
//! correctness gates — they make iteration on Verification
//! Definitions practical. The headed-browser debug toggle is the
//! `DUHEM_HEADED` env var (spec #151), not a flag.

mod browser_cmd;
mod contract_check;
mod dashboard;
mod describe_cmd;
mod environment;
mod export_cmd;
mod filter;
mod init;
mod inputs;
mod live_link;
mod live_progress;
mod mcp_cmd;
mod reporter;
mod reporter_config;
mod resolve;
mod run_cmd;
mod validate_cmd;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
        /// Action family for the scaffolded first check. `api`
        /// (default) is browser-free — `duhem run` needs only a
        /// network connection. `ui` scaffolds a browser-driven
        /// `ui/*` check (needs a one-time `duhem browser install`).
        #[arg(long = "kind", value_name = "api|ui", default_value = "api")]
        kind: String,
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
    /// List the built-in action catalog (`uses` + one-line summary).
    Actions,
    /// Print one action's contract: its `with:` fields and `outputs`.
    ///
    /// Version-exact ground truth for authoring a check — e.g.
    /// `duhem describe ui/assert-element` shows it produces `satisfied`.
    Describe {
        /// The action's `uses` string, e.g. `ui/assert-element`.
        uses: String,
    },
    /// Run an MCP (Model Context Protocol) server over stdio, exposing the
    /// action catalog, `describe`, and `validate` as tools — so a bare-chat
    /// agent can author + validate a VD with no repo checkout. (#251)
    Mcp,
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
    /// `--filter`, `--db`, `--reporter` are authoring-ergonomics
    /// flags (specs #23 / #189); none change the verdict on a
    /// non-filtered run.
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
        /// Evidence store (SQLite DB) the run is recorded into.
        /// Defaults to the working copy's project store under the
        /// duhem state dir (`DUHEM_HOME` honored); created and
        /// migrated if missing.
        #[arg(long = "db", value_name = "PATH")]
        db: Option<PathBuf>,
        /// Pin the run id instead of minting a fresh ULID. For
        /// fixtures and tests that need deterministic run URLs;
        /// single-leaf runs only (a manifest run has several leaves).
        #[arg(long = "run-id", value_name = "ID")]
        run_id: Option<String>,
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
        /// Failure-evidence capture for ui checks (spec #202):
        /// `on-failure` (the default) records a full-page screenshot
        /// and a DOM snapshot when a ui check ends with any non-pass
        /// assertion; `always` also captures the final state of
        /// passing ui checks; `off` disables capture. Captures land
        /// as `capture/*` artifacts on the check's evidence.
        #[arg(
            long = "capture",
            value_name = "on-failure|always|off",
            default_value = "on-failure"
        )]
        capture: duhem_runtime::CapturePolicy,
        /// Force live progress on stderr (spec #299). Terminals show
        /// the active check, step, expectation, and timeout budget;
        /// piped/CI output stays append-only. Auto-detected by default:
        /// on when stderr is a terminal, off when piped/CI. stdout is
        /// never touched — reporters stay machine-stable.
        #[arg(long = "live", default_value_t = false, conflicts_with = "no_live")]
        live: bool,
        /// Suppress live progress even on a TTY (spec #299).
        #[arg(long = "no-live", default_value_t = false)]
        no_live: bool,
        /// Open the dashboard's live run page in a browser (spec
        /// #305), once per invocation. Needs a resolvable dashboard —
        /// a serving `duhem dashboard` on this store, or
        /// DUHEM_DASHBOARD_URL; without one it warns and the run
        /// proceeds. DUHEM_OPENER overrides the platform opener.
        #[arg(long = "watch", default_value_t = false)]
        watch: bool,
        /// Also record a screencast video of each ui check (spec #215),
        /// kept under the same `--capture` policy as the screenshot/DOM.
        /// Opt-in and off by default: video blobs are large and ship to
        /// the hosted hub. Lands as a `capture/video` artifact. Recording
        /// must be enabled up front, so with `--capture on-failure` every
        /// ui check is recorded but only failing checks keep the file.
        #[arg(long = "capture-video", default_value_t = false)]
        capture_video: bool,
    },
    /// Browse run evidence in a read-only web dashboard.
    ///
    /// Shells out to the separate `duhem-dashboard` binary (specs
    /// #53 / #87); `dashboard.rs` owns binary resolution and the
    /// serve/export surface.
    Dashboard(dashboard::DashboardOpts),
    /// Provision the Playwright sidecar + Chromium for `ui/*` checks.
    ///
    /// `duhem browser install` installs the embedded sidecar's npm
    /// dependencies and the Chromium binary (spec #241). A distributed
    /// `duhem` carries the sidecar source but not `node_modules`/Chromium,
    /// so this is the one-time setup before `ui/*` checks can run.
    Browser(browser_cmd::BrowserOpts),
    /// Export one run from the store as a self-contained bundle
    /// (run header + wire-format event stream + artifacts) — the
    /// portability path (#189).
    Export {
        /// Run id (ULID) to export.
        run_id: String,
        /// Evidence store to read from. Defaults to the working
        /// copy's project store.
        #[arg(long = "db", value_name = "PATH")]
        db: Option<PathBuf>,
        /// Output directory (created if missing). Defaults to
        /// `duhem-export-<run-id>/`.
        #[arg(long = "out", value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Ship one run's bundle to a hub ingest endpoint (#194) —
    /// replication keyed by content hash, not dual-truth. No-op
    /// success with `--if-configured` when no hub is set.
    Ship {
        /// Run id (ULID) to ship.
        run_id: String,
        /// Evidence store to read from. Defaults to the working
        /// copy's project store.
        #[arg(long = "db", value_name = "PATH")]
        db: Option<PathBuf>,
        /// Hub ingest URL. Defaults to $DUHEM_HUB_URL; the bearer
        /// token comes from $DUHEM_HUB_TOKEN.
        #[arg(long = "hub-url", value_name = "URL")]
        hub_url: Option<String>,
        /// Exit 0 (skip) instead of erroring when no hub URL is
        /// configured — the CI ship step runs unconditionally.
        #[arg(long = "if-configured", default_value_t = false)]
        if_configured: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        None => ExitCode::SUCCESS,
        Some(Cmd::Init {
            path,
            pattern,
            kind,
            name,
            force,
        }) => init::run_init(path, &pattern, &kind, name, force),
        Some(Cmd::Actions) => describe_cmd::run_actions(),
        Some(Cmd::Describe { uses }) => describe_cmd::run_describe(&uses),
        Some(Cmd::Mcp) => mcp_cmd::run(),
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
        Some(Cmd::Browser(opts)) => browser_cmd::run(&opts),
        Some(Cmd::Export { run_id, db, out }) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(export_cmd::run_export(
                &run_id,
                db.as_deref(),
                out.as_deref(),
            ))
        }
        Some(Cmd::Ship {
            run_id,
            db,
            hub_url,
            if_configured,
        }) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(export_cmd::run_ship(
                &run_id,
                db.as_deref(),
                hub_url.as_deref(),
                if_configured,
            ))
        }
        Some(Cmd::Run {
            path,
            file,
            inputs,
            filter,
            db,
            run_id,
            reporter,
            environment,
            dry_run,
            no_env_up,
            keep_env,
            live,
            no_live,
            watch,
            capture,
            capture_video,
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
            rt.block_on(run_cmd::run_command(run_cmd::RunArgs {
                path,
                file,
                inputs,
                filter,
                db,
                run_id,
                reporter: resolved_reporter,
                environment,
                dry_run,
                no_env_up,
                keep_env,
                // Tri-state: forced on / forced off / auto (TTY).
                live: match (live, no_live) {
                    (true, _) => Some(true),
                    (_, true) => Some(false),
                    _ => None,
                },
                watch,
                capture,
                capture_video,
            }))
        }
    }
}

#[cfg(test)]
mod leaf_name_tests {
    use crate::run_cmd::leaf_name;
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
    use crate::resolve::resolve_inputs;
    use crate::run_cmd::parse_truthy;
    use clap::Parser;
    use duhem_schema::{InputDecl, VerificationDefinition};
    use std::collections::BTreeMap;

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
