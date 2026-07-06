//! `duhem-dashboard` binary: serve mode (default) and `export`.
//!
//! Kept as a separate binary from `duhem` (the #53 alignment
//! decision) so the web server + SPA toolchain stays out of the core
//! CLI; `duhem dashboard` shells out to this (#87).
//!
//! The dashboard opens the evidence store **read-only** (#189): by
//! default the working copy's project DB
//! (`$XDG_STATE_HOME/duhem/projects/<slug>/duhem.db`, `DUHEM_HOME`
//! honored), or an explicit `--db` path.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use duhem_dashboard::{DEFAULT_PORT, EvidenceReader, export, router};
use duhem_evidence::SqliteStore;

/// Read-only web dashboard over the Duhem evidence store.
#[derive(Debug, Parser)]
#[command(name = "duhem-dashboard", version)]
struct Cli {
    /// Evidence store (SQLite DB) to read runs from. Defaults to the
    /// working copy's project store under the duhem state dir.
    #[arg(long = "db", value_name = "PATH", global = true)]
    db: Option<PathBuf>,

    /// Listen port (serve mode).
    #[arg(long = "port", value_name = "PORT", default_value_t = DEFAULT_PORT)]
    port: u16,

    /// Listen host (serve mode). Loopback by default: the MVP has no
    /// auth, exposure is the operator's deliberate choice.
    #[arg(long = "host", value_name = "HOST", default_value = "127.0.0.1")]
    host: String,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Render a self-contained static site (SPA bundle + JSON
    /// snapshots + artifacts) for upload to a file host.
    Export {
        /// Output directory (created if missing).
        #[arg(long = "out", value_name = "DIR")]
        out: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    rt.block_on(run(cli))
}

async fn run(cli: Cli) -> ExitCode {
    let db_path = match &cli.db {
        Some(p) => p.clone(),
        None => match std::env::current_dir()
            .map_err(duhem_evidence::StoreError::Io)
            .and_then(|cwd| duhem_evidence::project_db_path(&cwd))
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("resolve store: {e}");
                return ExitCode::FAILURE;
            }
        },
    };

    // Read-only lens: the dashboard can never mutate the store.
    let store = match SqliteStore::open_read_only(&db_path).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("open store: {e}");
            return ExitCode::FAILURE;
        }
    };
    let reader = EvidenceReader::new(Arc::new(store));

    match cli.cmd {
        Some(Cmd::Export { out }) => match export(&reader, &out).await {
            Ok(stats) => {
                println!(
                    "exported {} run(s), {} check page(s), {} artifact(s), {} SPA file(s) to {}",
                    stats.runs,
                    stats.checks,
                    stats.artifacts,
                    stats.spa_files,
                    out.display()
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("export: {e:#}");
                ExitCode::FAILURE
            }
        },
        None => serve(reader, &db_path, &cli.host, cli.port).await,
    }
}

async fn serve(
    reader: EvidenceReader,
    db_path: &std::path::Path,
    host: &str,
    port: u16,
) -> ExitCode {
    let listener = match tokio::net::TcpListener::bind((host, port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind {host}:{port}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    println!(
        "duhem dashboard listening on http://{addr}/ (store: {})",
        db_path.display()
    );
    match axum::serve(listener, router(reader)).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("serve: {e}");
            ExitCode::FAILURE
        }
    }
}
