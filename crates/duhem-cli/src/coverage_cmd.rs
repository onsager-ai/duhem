//! Store-limited reporting of declared definitions and their latest recorded run.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Args, ValueEnum};
use duhem_evidence::{RunRecord, RunStatus, SqliteStore, Store};
use duhem_schema::Loaded;
use serde::Serialize;

const LIMIT: &str = "Only runs recorded in this supplied store are visible. No recorded run is not proof that an invocation never happened, a body executed, or a gate passed. Definitions match canonical source paths. Supply --base with the original invocation directory to resolve relative records; original directories and relocated paths are not guessed.";
const CANDIDATES: [&str; 4] = ["duhem.yml", "duhem.yaml", ".duhem.yml", ".duhem.yaml"];

#[derive(Debug, Args)]
pub(crate) struct CoverageArgs {
    /// Leaf, manifest, or directory tree containing conventional Duhem files.
    pub path: PathBuf,
    /// Existing evidence store to inspect read-only; no implicit store selection.
    #[arg(long, value_name = "PATH")]
    pub db: PathBuf,
    /// Original invocation directory for relative source paths in this store.
    /// Omit only when recorded paths are absolute; the store key is not reversible.
    #[arg(long, value_name = "DIR")]
    pub base: Option<PathBuf>,
    /// Output representation. Absence of runs never changes the exit code.
    #[arg(long, value_enum, default_value = "text")]
    pub format: Format,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Format {
    Text,
    Json,
}

#[derive(Serialize)]
struct CoverageReport {
    schema_version: &'static str,
    store: PathBuf,
    scope: &'static str,
    base: Option<PathBuf>,
    unmatched_runs: UnmatchedRuns,
    definitions: Vec<DefinitionRecord>,
}

#[derive(Default, Serialize)]
struct UnmatchedRuns {
    total: usize,
    relative: usize,
    without_base: usize,
}

#[derive(Serialize)]
struct DefinitionRecord {
    verification: PathBuf,
    status: &'static str,
    // Explicit null is distinct from a recorded run with no verdict yet.
    last_recorded_run: Option<LastRun>,
}

#[derive(Serialize)]
struct LastRun {
    run_id: String,
    started_at: chrono::DateTime<chrono::Utc>,
    status: &'static str,
    verdict: Option<String>,
}

pub(crate) async fn run(args: &CoverageArgs) -> ExitCode {
    match report(args).await {
        Ok(report) => {
            match args.format {
                Format::Json => println!(
                    "{}",
                    serde_json::to_string(&report).expect("report serializes")
                ),
                Format::Text => {
                    println!("Store: {}\n{}", report.store.display(), report.scope);
                    if let Some(base) = &report.base {
                        println!("Relative-record base: {}", base.display());
                    }
                    println!("\nDefinition\tLatest recorded run start\tRun status\tVerdict");
                    for definition in &report.definitions {
                        match &definition.last_recorded_run {
                            Some(run) => println!(
                                "{}\t{}\t{}\t{}",
                                definition.verification.display(),
                                run.started_at.to_rfc3339(),
                                run.status,
                                run.verdict.as_deref().unwrap_or("no recorded verdict")
                            ),
                            None => println!(
                                "{}\tno_recorded_run\t—\t—",
                                definition.verification.display()
                            ),
                        }
                    }
                    let absent = report
                        .definitions
                        .iter()
                        .filter(|definition| definition.last_recorded_run.is_none())
                        .count();
                    println!(
                        "\n{absent} of {} definitions have no recorded run in this store.",
                        report.definitions.len()
                    );
                    println!(
                        "{} recorded runs could not be matched to a definition ({} relative; {} without --base).",
                        report.unmatched_runs.total,
                        report.unmatched_runs.relative,
                        report.unmatched_runs.without_base
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("coverage: {error}");
            ExitCode::from(2)
        }
    }
}

async fn report(args: &CoverageArgs) -> Result<CoverageReport, String> {
    let mut paths = BTreeSet::new();
    collect(&args.path, &mut paths)?;
    if paths.is_empty() {
        return Err("no declared definitions found; supply a leaf file, manifest, or directory containing conventional Duhem files".into());
    }
    let store = SqliteStore::open_read_only(&args.db)
        .await
        .map_err(|error| error.to_string())?;
    let mut latest: BTreeMap<PathBuf, RunRecord> = BTreeMap::new();
    let base = args
        .base
        .as_ref()
        .map(|base| {
            let base = base
                .canonicalize()
                .map_err(|error| format!("--base {}: {error}", base.display()))?;
            if !base.is_dir() {
                return Err(format!("--base is not a directory: {}", base.display()));
            }
            Ok(base)
        })
        .transpose()?;
    let mut unmatched_runs = UnmatchedRuns::default();
    for run in store.list_runs().await.map_err(|error| error.to_string())? {
        let path = Path::new(&run.verification);
        let relative = path.is_relative();
        let resolved = if relative {
            base.as_ref().map(|base| base.join(path))
        } else {
            Some(path.to_path_buf())
        };
        let matched = resolved
            .and_then(|path| path.canonicalize().ok())
            .filter(|path| paths.contains(path));
        let Some(path) = matched else {
            unmatched_runs.total += 1;
            if relative {
                unmatched_runs.relative += 1;
            }
            if relative && base.is_none() {
                unmatched_runs.without_base += 1;
            }
            continue;
        };
        if latest.get(&path).is_none_or(|previous| {
            (run.started_at, &run.run_id) > (previous.started_at, &previous.run_id)
        }) {
            latest.insert(path, run);
        }
    }
    let definitions = paths
        .into_iter()
        .map(|verification| {
            let run = latest.remove(&verification);
            DefinitionRecord {
                verification,
                status: if run.is_some() {
                    "recorded_run"
                } else {
                    "no_recorded_run"
                },
                last_recorded_run: run.map(|run| LastRun {
                    run_id: run.run_id,
                    started_at: run.started_at,
                    status: match run.status {
                        RunStatus::Running => "running",
                        RunStatus::Finished => "finished",
                        RunStatus::Aborted => "aborted",
                        RunStatus::Orphaned => "orphaned",
                    },
                    verdict: run.verdict.map(|verdict| verdict.to_string()),
                }),
            }
        })
        .collect();
    Ok(CoverageReport {
        schema_version: "1",
        store: args.db.clone(),
        scope: LIMIT,
        base,
        unmatched_runs,
        definitions,
    })
}

fn collect(path: &Path, paths: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        return collect_file(path, paths);
    }
    if !path.is_dir() {
        return Err(format!("not a file or directory: {}", path.display()));
    }
    // A declaration root owns its subtree; loading its manifest, rather than
    // scraping sibling YAML, retains includes, globs, and arbitrary leaf names.
    if let Some(file) = CANDIDATES
        .iter()
        .map(|name| path.join(name))
        .find(|file| file.is_file())
    {
        return collect_file(&file, paths);
    }
    for entry in std::fs::read_dir(path).map_err(|error| format!("{}: {error}", path.display()))? {
        let entry = entry.map_err(|error| error.to_string())?;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_dir()
            && !matches!(
                entry.file_name().to_str(),
                Some(".git" | "node_modules" | "target")
            )
        {
            collect(&entry.path(), paths)?;
        }
    }
    Ok(())
}

fn collect_file(path: &Path, paths: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    match duhem_schema::load_for_run(path).map_err(|error| error.to_string())? {
        Loaded::Leaf { path, .. } => {
            paths.insert(path.canonicalize().map_err(|error| error.to_string())?);
        }
        Loaded::Manifest {
            leaves, warnings, ..
        } => {
            for warning in warnings {
                eprintln!("coverage: {warning}");
            }
            for leaf in leaves {
                paths.insert(
                    leaf.path
                        .canonicalize()
                        .map_err(|error| error.to_string())?,
                );
            }
        }
    }
    Ok(())
}
