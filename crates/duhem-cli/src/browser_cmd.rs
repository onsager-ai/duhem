//! `duhem browser install` — provision the Playwright sidecar + Chromium.
//!
//! `ui/*` checks drive a Node Playwright sidecar. The `duhem` binary
//! embeds the sidecar *source* (rust-embed, `duhem-actions::browser`), but
//! its `playwright` npm dependency and the Chromium binary are large and
//! machine-specific, so they're installed on demand here rather than
//! shipped. This materializes the embedded sidecar into the user cache dir
//! (when running a distributed binary) and runs `npm ci` +
//! `npx playwright install [--with-deps] chromium` in it. Idempotent.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::{Args, Subcommand};

use duhem_actions::browser::{materialize_sidecar, sidecar_dir};

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

pub fn run(opts: &BrowserOpts) -> ExitCode {
    match &opts.cmd {
        BrowserCmd::Install { with_deps } => install(*with_deps),
    }
}

fn install(with_deps: bool) -> ExitCode {
    let dir = match sidecar_dir_for_install() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("browser install: could not prepare the sidecar: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("Sidecar: {}", dir.display());

    if let Err(code) = check_node() {
        return code;
    }

    // 1. Sidecar node deps (playwright). `npm ci` is exact + reproducible
    //    against the embedded lockfile; fall back to `npm install`.
    println!("→ installing sidecar dependencies (npm ci)…");
    if !run_in("npm", &["ci"], &dir) {
        eprintln!("  npm ci failed; retrying with npm install…");
        if !run_in("npm", &["install"], &dir) {
            eprintln!("browser install: npm install failed in {}", dir.display());
            return ExitCode::FAILURE;
        }
    }

    // 2. The Chromium browser binary.
    let mut pw_args = vec!["--yes", "playwright", "install"];
    if with_deps {
        pw_args.push("--with-deps");
    }
    pw_args.push("chromium");
    println!(
        "→ installing Chromium (npx playwright install{})…",
        if with_deps { " --with-deps" } else { "" }
    );
    if !run_in("npx", &pw_args, &dir) {
        eprintln!("browser install: npx playwright install chromium failed");
        return ExitCode::FAILURE;
    }

    println!("✓ Browser ready — `ui/*` checks can now run.");
    ExitCode::SUCCESS
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

fn check_node() -> Result<(), ExitCode> {
    match Command::new("node").arg("--version").output() {
        Ok(out) => {
            let v = String::from_utf8_lossy(&out.stdout);
            let major = v
                .trim()
                .trim_start_matches('v')
                .split('.')
                .next()
                .and_then(|s| s.parse::<u32>().ok());
            match major {
                Some(m) if m >= 20 => Ok(()),
                Some(m) => {
                    eprintln!(
                        "browser install: Node {m} is too old; the Playwright sidecar needs Node >= 20."
                    );
                    Err(ExitCode::FAILURE)
                }
                None => {
                    eprintln!("browser install: could not parse `node --version`.");
                    Err(ExitCode::FAILURE)
                }
            }
        }
        Err(e) => {
            eprintln!(
                "browser install: Node.js not found (`node --version`: {e}). Install Node >= 20."
            );
            Err(ExitCode::FAILURE)
        }
    }
}

fn run_in(program: &str, args: &[&str], dir: &Path) -> bool {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
