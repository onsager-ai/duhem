//! `duhem export` (#189) and `duhem ship` (#194): two destinations
//! for one format — the [`duhem_evidence::RunBundle`].
//!
//! `export` writes the human-browsable directory layout; `ship`
//! POSTs the canonical JSON envelope to a hub's ingest endpoint,
//! keyed by the bundle's content hash so re-shipping the same run is
//! a server-side no-op. The hub server itself is the closed-source
//! sibling (#188 open-core seam) — this repo owns only the client
//! and the wire contract (`tests/bundle_contract.rs` pins it).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use duhem_evidence::{RunBundle, SqliteStore};

/// Environment variables the ship step reads. The URL names the hub
/// ingest endpoint; the token authenticates (the OAuth/session flow
/// that mints it is the hub's concern).
pub const HUB_URL_ENV: &str = "DUHEM_HUB_URL";
pub const HUB_TOKEN_ENV: &str = "DUHEM_HUB_TOKEN";

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

/// `duhem ship <run-id>`: POST the bundle to the hub. Without a hub
/// URL configured this is a clean no-op success — the CI ship step
/// can run unconditionally and only bites when the operator opted in.
pub async fn run_ship(
    run_id: &str,
    db: Option<&Path>,
    hub_url: Option<&str>,
    quiet_unconfigured: bool,
) -> ExitCode {
    let url = hub_url
        .map(str::to_string)
        .or_else(|| std::env::var(HUB_URL_ENV).ok().filter(|v| !v.is_empty()));
    let Some(url) = url else {
        if quiet_unconfigured {
            println!("ship: no hub configured ({HUB_URL_ENV} unset); skipping");
            return ExitCode::SUCCESS;
        }
        eprintln!("ship: no hub URL — pass --hub-url or set {HUB_URL_ENV}");
        return ExitCode::FAILURE;
    };

    let bundle = match open_and_bundle(run_id, db).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ship: {e}");
            return ExitCode::FAILURE;
        }
    };
    match ship_bundle(&bundle, &url, std::env::var(HUB_TOKEN_ENV).ok().as_deref()).await {
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
/// Idempotency: the `X-Duhem-Content-Hash` header carries the
/// bundle's hash; a hub that has it already answers 200/409 without
/// re-storing. 2xx = shipped.
pub async fn ship_bundle(
    bundle: &RunBundle,
    url: &str,
    token: Option<&str>,
) -> Result<String, String> {
    let body = bundle.wire_bytes().map_err(|e| e.to_string())?;
    let hash = bundle.content_hash().map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
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
    let res = req.send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        return Err(format!("hub answered {status}: {}", text.trim()));
    }
    Ok(hash)
}
