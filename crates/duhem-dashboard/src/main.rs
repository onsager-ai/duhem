//! `duhem-dashboard` binary: serve mode (default) and `export`.
//!
//! Kept as a separate binary from `duhem` (the #53 alignment
//! decision) so the web server + SPA toolchain stays out of the core
//! CLI; `duhem dashboard` shells out to this (#87).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use duhem_dashboard::{DEFAULT_EVIDENCE_DIR, DEFAULT_PORT, EvidenceReader, export, router};

/// Read-only web dashboard over Duhem run evidence.
#[derive(Debug, Parser)]
#[command(name = "duhem-dashboard", version)]
struct Cli {
    /// Evidence directory to read runs from.
    #[arg(long = "evidence-dir", value_name = "PATH", default_value = DEFAULT_EVIDENCE_DIR, global = true)]
    evidence_dir: PathBuf,

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
    let reader = EvidenceReader::new(&cli.evidence_dir);

    match cli.cmd {
        Some(Cmd::Export { out }) => match export(&reader, &out) {
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
        None => {
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
            rt.block_on(serve(reader, &cli.host, cli.port))
        }
    }
}

async fn serve(reader: EvidenceReader, host: &str, port: u16) -> ExitCode {
    let evidence = reader.root().display().to_string();
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
    println!("duhem dashboard listening on http://{addr}/ (evidence: {evidence})");
    match axum::serve(listener, router(reader)).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("serve: {e}");
            ExitCode::FAILURE
        }
    }
}
