//! `duhem dashboard` (#87): the operator surface for the
//! `duhem-dashboard` binary.
//!
//! The dashboard ships as a separate binary (the #53 alignment
//! decision — its web server + SPA toolchain stays out of the core
//! CLI), so this module is a thin process wrapper: resolve the
//! binary, forward the flags, propagate the exit code. Resolution
//! order: `DUHEM_DASHBOARD_BIN` override → sibling of the running
//! `duhem` executable → `PATH`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::{Args, Subcommand};

pub const BIN_ENV_OVERRIDE: &str = "DUHEM_DASHBOARD_BIN";
const BIN_NAME: &str = if cfg!(windows) {
    "duhem-dashboard.exe"
} else {
    "duhem-dashboard"
};

/// Where to find `duhem-dashboard`. `None` means "spawn by name and
/// let the OS search `PATH`".
pub fn resolve_binary(env_override: Option<&str>, current_exe: Option<&Path>) -> PathBuf {
    if let Some(path) = env_override.filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(sibling) = current_exe
        .and_then(|exe| exe.parent())
        .map(|dir| dir.join(BIN_NAME))
        .filter(|p| p.is_file())
    {
        return sibling;
    }
    PathBuf::from(BIN_NAME)
}

/// The `duhem dashboard` clap surface (#87).
#[derive(Debug, Args)]
pub struct DashboardOpts {
    /// Evidence store (SQLite DB) to read runs from. Defaults to the
    /// working copy's project store under the duhem state dir.
    #[arg(long = "db", value_name = "PATH", global = true)]
    pub db: Option<PathBuf>,
    /// Listen port (serve mode; default 7878).
    #[arg(long = "port", value_name = "PORT")]
    pub port: Option<u16>,
    /// Listen host (serve mode; default 127.0.0.1).
    #[arg(long = "host", value_name = "HOST")]
    pub host: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<DashboardCmd>,
}

#[derive(Debug, Subcommand)]
pub enum DashboardCmd {
    /// Render a self-contained static site (SPA + JSON snapshots +
    /// artifacts) for upload to a file host (S3, GH Pages).
    Export {
        /// Output directory (created if missing).
        #[arg(long = "out", value_name = "DIR")]
        out: PathBuf,
    },
}

/// Arguments for the child, mirroring the `duhem dashboard` surface.
pub struct DashboardArgs {
    pub db: Option<PathBuf>,
    pub port: Option<u16>,
    pub host: Option<String>,
    /// `Some(out)` selects export mode.
    pub export_out: Option<PathBuf>,
}

impl From<DashboardOpts> for DashboardArgs {
    fn from(opts: DashboardOpts) -> Self {
        let DashboardOpts {
            db,
            port,
            host,
            cmd,
        } = opts;
        DashboardArgs {
            db,
            port,
            host,
            export_out: cmd.map(|DashboardCmd::Export { out }| out),
        }
    }
}

pub fn forward_args(args: &DashboardArgs) -> Vec<String> {
    let mut argv = Vec::new();
    if let Some(db) = &args.db {
        argv.push("--db".into());
        argv.push(db.display().to_string());
    }
    match &args.export_out {
        Some(out) => {
            argv.push("export".into());
            argv.push("--out".into());
            argv.push(out.display().to_string());
        }
        None => {
            if let Some(port) = args.port {
                argv.push("--port".into());
                argv.push(port.to_string());
            }
            if let Some(host) = &args.host {
                argv.push("--host".into());
                argv.push(host.clone());
            }
        }
    }
    argv
}

/// Spawn `duhem-dashboard` and wait. The child shares our stdio and
/// process group, so Ctrl-C reaches it directly; we just reflect its
/// exit status.
pub fn run(args: &DashboardArgs) -> ExitCode {
    let env_override = std::env::var(BIN_ENV_OVERRIDE).ok();
    let current_exe = std::env::current_exe().ok();
    let bin = resolve_binary(env_override.as_deref(), current_exe.as_deref());

    let status = Command::new(&bin).args(forward_args(args)).status();
    match status {
        Ok(s) => match s.code() {
            Some(0) => ExitCode::SUCCESS,
            Some(code) => ExitCode::from(code.clamp(0, 255) as u8),
            // Terminated by signal (Ctrl-C path): mirror failure.
            None => ExitCode::FAILURE,
        },
        Err(e) => {
            eprintln!(
                "duhem dashboard: cannot launch `{}`: {e}\n\
                 install the duhem-dashboard binary next to `duhem`, put it on PATH, \
                 or point {BIN_ENV_OVERRIDE} at it",
                bin.display()
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        let bin = resolve_binary(
            Some("/opt/duhem-dashboard"),
            Some(Path::new("/usr/bin/duhem")),
        );
        assert_eq!(bin, PathBuf::from("/opt/duhem-dashboard"));
    }

    #[test]
    fn empty_override_is_ignored() {
        let bin = resolve_binary(Some(""), None);
        assert_eq!(bin, PathBuf::from(BIN_NAME));
    }

    #[test]
    fn sibling_is_used_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let sibling = tmp.path().join(BIN_NAME);
        std::fs::write(&sibling, b"#!/bin/sh\n").unwrap();
        let exe = tmp.path().join("duhem");
        let bin = resolve_binary(None, Some(&exe));
        assert_eq!(bin, sibling);
    }

    #[test]
    fn falls_back_to_path_lookup() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("duhem");
        let bin = resolve_binary(None, Some(&exe));
        assert_eq!(bin, PathBuf::from(BIN_NAME));
    }

    #[test]
    fn serve_flags_forward_and_export_selects_the_subcommand() {
        let serve = DashboardArgs {
            db: Some(PathBuf::from("state/duhem.db")),
            port: Some(8080),
            host: Some("0.0.0.0".into()),
            export_out: None,
        };
        assert_eq!(
            forward_args(&serve),
            vec![
                "--db",
                "state/duhem.db",
                "--port",
                "8080",
                "--host",
                "0.0.0.0"
            ]
        );

        let export = DashboardArgs {
            db: None,
            port: Some(8080), // serve-only flag: not forwarded in export mode
            host: None,
            export_out: Some(PathBuf::from("site/")),
        };
        assert_eq!(forward_args(&export), vec!["export", "--out", "site/"]);
    }
}
