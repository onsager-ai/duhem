//! `duhem export` (#189) and `duhem ship` (#194): two destinations
//! for one format — the [`duhem_evidence::RunBundle`].
//!
//! `export` writes the human-browsable directory layout; `ship`
//! POSTs the canonical JSON envelope to a server's ingest endpoint,
//! keyed by the bundle's content hash so re-shipping the same run is
//! a server-side no-op.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use duhem_evidence::{RunBundle, SqliteStore};

/// Environment variables used by run auto-send and `duhem ship`.
pub const SERVER_URL_ENV: &str = "DUHEM_SERVER_URL";
pub const SERVER_TOKEN_ENV: &str = "DUHEM_SERVER_TOKEN";
pub const HUB_URL_ENV: &str = "DUHEM_HUB_URL";
pub const HUB_TOKEN_ENV: &str = "DUHEM_HUB_TOKEN";

static DEPRECATION_WARNED: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashSet<&'static str>>,
> = std::sync::OnceLock::new();

pub fn warn_deprecated_env(old: &'static str, new: &'static str) {
    let warned = DEPRECATION_WARNED.get_or_init(Default::default);
    let mut warned = warned.lock().unwrap();
    if warned.insert(old) {
        eprintln!("warning: {old} is deprecated; use {new}");
    }
}

fn env_value(new: &'static str, old: &'static str) -> Option<String> {
    if let Ok(value) = std::env::var(new)
        && !value.is_empty()
    {
        return Some(value);
    }
    let value = std::env::var(old).ok().filter(|value| !value.is_empty())?;
    warn_deprecated_env(old, new);
    Some(value)
}

fn upload_url(explicit: Option<&str>) -> Option<String> {
    if let Some(url) = explicit.filter(|value| !value.is_empty()) {
        return Some(url.to_string());
    }
    if let Ok(url) = std::env::var(SERVER_URL_ENV)
        && !url.is_empty()
    {
        return Some(format!("{}/api/ingest", url.trim_end_matches('/')));
    }
    let url = std::env::var(HUB_URL_ENV)
        .ok()
        .filter(|value| !value.is_empty())?;
    warn_deprecated_env(HUB_URL_ENV, SERVER_URL_ENV);
    Some(url)
}

async fn open_and_bundle(run_id: &str, db: Option<&Path>) -> Result<RunBundle, String> {
    let db_path = match db {
        Some(p) => p.to_path_buf(),
        None => {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            duhem_evidence::project_db_path(&cwd).map_err(|e| e.to_string())?
        }
    };
    // Read-only: neither export nor ship may mutate the store.
    let store = SqliteStore::open_read_only(&db_path)
        .await
        .map_err(|e| e.to_string())?;
    RunBundle::from_store(&store, run_id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn run_export(run_id: &str, db: Option<&Path>, out: Option<&Path>) -> ExitCode {
    let bundle = match open_and_bundle(run_id, db).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("export: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out_dir = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(format!("duhem-export-{run_id}")));
    match bundle.write_dir(&out_dir) {
        Ok(()) => {
            println!("exported run {run_id} to {}", out_dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("export: {e}");
            ExitCode::FAILURE
        }
    }
}

/// POST a run bundle to the configured server. Without a URL this is a clean no-op when requested.
pub async fn run_ship(
    run_id: &str,
    db: Option<&Path>,
    hub_url: Option<&str>,
    quiet_unconfigured: bool,
) -> ExitCode {
    let url = upload_url(hub_url);
    let Some(url) = url else {
        if quiet_unconfigured {
            println!("ship: no server configured ({SERVER_URL_ENV} unset); skipping");
            return ExitCode::SUCCESS;
        }
        eprintln!("ship: no server URL — pass --hub-url or set {SERVER_URL_ENV}");
        return ExitCode::FAILURE;
    };

    let db_path = match db {
        Some(path) => Ok(path.to_path_buf()),
        None => std::env::current_dir()
            .map_err(duhem_evidence::StoreError::from)
            .and_then(|cwd| duhem_evidence::project_db_path(&cwd)),
    };
    let store = match db_path {
        Ok(path) => SqliteStore::open_read_only(path).await,
        Err(error) => Err(error),
    };
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            eprintln!("ship: {error}");
            return ExitCode::FAILURE;
        }
    };
    match ship_tree(&store, &[run_id.to_string()], &url).await {
        Ok(hash) => {
            println!("shipped run {run_id} to {url} (content hash {hash})");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ship: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The ingest client (#194): one POST of the canonical envelope.
pub async fn ship_bundle(
    bundle: &RunBundle,
    url: &str,
    token: Option<&str>,
) -> Result<String, String> {
    let body = bundle.wire_bytes().map_err(|e| e.to_string())?;
    let hash = bundle.content_hash().map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client
        .post(url)
        .header("content-type", "application/json")
        .header("x-duhem-bundle-version", bundle.bundle_version.to_string())
        .header("x-duhem-content-hash", &hash)
        .body(body);
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    let res = req.send().await.map_err(|e| e.without_url().to_string())?;
    if !res.status().is_success() {
        return Err(format!("hub answered {}", res.status()));
    }
    Ok(hash)
}

/// Parents first, then every descendant (including nested invocations).
async fn ship_tree(
    store: &dyn duhem_evidence::Store,
    roots: &[String],
    url: &str,
) -> Result<String, String> {
    let records = store.list_runs().await.map_err(|e| e.to_string())?;
    let mut ordered = Vec::new();
    let mut stack = roots.to_vec();
    while let Some(id) = stack.pop() {
        if ordered.contains(&id) {
            continue;
        }
        ordered.push(id.clone());
        stack.extend(
            records
                .iter()
                .filter(|record| record.lineage.parent_run_id.as_deref() == Some(id.as_str()))
                .map(|record| record.run_id.clone()),
        );
    }
    // Include ancestors so one manually shipped child can reconstruct its tree.
    let mut index = 0;
    while index < ordered.len() {
        if let Some(parent) = records
            .iter()
            .find(|record| record.run_id == ordered[index])
            .and_then(|record| record.lineage.parent_run_id.as_ref())
            && !ordered.contains(parent)
        {
            ordered.push(parent.clone());
        }
        index += 1;
    }
    let mut sent: Vec<String> = Vec::new();
    let token = env_value(SERVER_TOKEN_ENV, HUB_TOKEN_ENV);
    let mut last_hash = String::new();
    while sent.len() < ordered.len() {
        let id = ordered
            .iter()
            .find(|id| {
                !sent.contains(*id)
                    && records
                        .iter()
                        .find(|record| &record.run_id == *id)
                        .is_some_and(|record| {
                            record
                                .lineage
                                .parent_run_id
                                .as_ref()
                                .is_none_or(|parent| sent.contains(parent))
                        })
            })
            .ok_or_else(|| "missing parent or cyclic run lineage".to_string())?;
        let bundle = RunBundle::from_store(store, id)
            .await
            .map_err(|e| e.to_string())?;
        last_hash = ship_bundle(&bundle, url, token.as_deref()).await?;
        sent.push(id.clone());
    }
    Ok(last_hash)
}

/// Delivery is best-effort and never enters the verdict or reporter fold.
pub async fn auto_send(store: &dyn duhem_evidence::Store, roots: &[String]) {
    let Some(url) = upload_url(None) else {
        return;
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        ship_tree(store, roots, &url),
    )
    .await;
    let error = match result {
        Ok(Ok(_)) => return,
        Ok(Err(error)) => error,
        Err(_) => "delivery timed out".into(),
    };
    eprintln!("warning: auto-send: {}", error.replace(['\r', '\n'], " "));
}
