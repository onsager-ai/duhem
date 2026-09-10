//! `duhem browser install` — provision the Playwright sidecar + Chromium.
//!
//! `ui/*` checks drive a Node Playwright sidecar. The `duhem` binary
//! embeds the sidecar *source* (rust-embed, `duhem-actions::browser`), but
//! its `playwright` npm dependency and the Chromium binary are large and
//! machine-specific, so they're installed on demand here rather than
//! shipped. This materializes the embedded sidecar into the user cache dir
//! (when running a distributed binary) and runs `npm ci` +
//! `npx playwright install [--with-deps] chromium` in it. Idempotent.
//!
//! The install mechanics (the npm step, the distro-refusal retry, the
//! cross-process lock) are shared with `duhem run`'s auto-provision via
//! [`duhem_actions::browser_provision::install_browser`] (#505); this
//! module is the clap surface, the human-friendly progress voice, and the
//! `--with-deps` policy choice that only the explicit command may make.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};

use duhem_actions::browser::{check_node, materialize_sidecar, sidecar_dir};
use duhem_actions::browser_provision::{
    HOST_PLATFORM_OVERRIDE_ENV, HOST_PLATFORM_OVERRIDE_VALUE, InstallPolicy, InstallProgress,
    install_browser,
};

/// `duhem browser …` clap surface.
#[derive(Debug, Args)]
pub struct BrowserOpts {
    #[command(subcommand)]
    pub cmd: BrowserCmd,
}

#[derive(Debug, Subcommand)]
pub enum BrowserCmd {
    /// Install the Playwright sidecar dependencies + Chromium so `ui/*`
    /// checks can run. Idempotent; safe to re-run.
    Install {
        /// Also install the OS libraries Chromium needs
        /// (`playwright install --with-deps`; may prompt for sudo).
        /// Use in CI images.
        #[arg(long = "with-deps", default_value_t = false)]
        with_deps: bool,
    },
}

pub async fn run(opts: &BrowserOpts) -> ExitCode {
    match &opts.cmd {
        BrowserCmd::Install { with_deps } => install(*with_deps).await,
    }
}

async fn install(with_deps: bool) -> ExitCode {
    let dir = match sidecar_dir_for_install() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("browser install: could not prepare the sidecar: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("Sidecar: {}", dir.display());

    if let Err(e) = check_node().await {
        eprintln!("browser install: {e}");
        return ExitCode::FAILURE;
    }

    let policy = InstallPolicy {
        with_deps,
        // Unlike auto-provision, always reinstall — this command is the
        // "make it work" hammer and documents itself as "idempotent;
        // safe to re-run".
        skip_npm_when_usable: false,
        // Unlike auto-provision, echo subprocess output live — a user
        // who typed this command should watch the ~110 MiB Chromium
        // download progress, not stare at silence for minutes.
        stream_output: true,
        progress: &CliProgress,
    };
    match install_browser(&dir, &policy).await {
        Ok(()) => {
            println!("✓ Browser ready — `ui/*` checks can now run.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("browser install: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Friendly stdout progress for the explicit command — a user who typed
/// `duhem browser install` wants to watch it work, unlike auto-provision's
/// terse `[duhem]` stderr lines (`ProvisionProgress` in
/// `duhem_actions::browser_provision`). These are just the framing lines
/// around each step; the subprocess's own output is additionally
/// streamed live (`InstallPolicy::stream_output: true`, set in
/// `install` above) since the Chromium download is the longest step and
/// silence there would read as a hang. On failure, `install_browser`'s
/// returned error also folds in the captured text, so it reaches the
/// user even if something scrolled past.
struct CliProgress;

impl InstallProgress for CliProgress {
    fn npm_start(&self) {
        println!("→ installing sidecar dependencies (npm ci)…");
    }

    fn npm_retry(&self) {
        eprintln!("  npm ci failed; retrying with npm install…");
    }

    fn chromium_start(&self, with_deps: bool) {
        println!(
            "→ installing Chromium (npx playwright install{})…",
            if with_deps { " --with-deps" } else { "" }
        );
    }

    fn distro_retry(&self) {
        // A pinned Playwright older than the host distro refuses with
        // `does not support chromium on <distro>`, though the prebuilt
        // ubuntu24.04 Chromium runs fine on newer releases. Retry once
        // forcing that build before giving up (#295).
        eprintln!(
            "  Chromium install failed; retrying with a compatible-OS override ({HOST_PLATFORM_OVERRIDE_ENV}={HOST_PLATFORM_OVERRIDE_VALUE})…"
        );
    }
}

/// The directory the runtime resolves the sidecar to, materializing the
/// embedded copy when there's no source tree. Mirrors
/// `browser::sidecar_dir` but surfaces materialization errors.
fn sidecar_dir_for_install() -> std::io::Result<PathBuf> {
    let dir = sidecar_dir();
    if dir.join("index.mjs").exists() {
        Ok(dir)
    } else {
        materialize_sidecar()
    }
}
