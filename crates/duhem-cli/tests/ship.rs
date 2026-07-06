//! `duhem ship` integration tests (#194): the ingest client POSTs
//! the canonical bundle with its idempotency headers, no-ops cleanly
//! when unconfigured, and errors loudly when pointed nowhere.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_duhem"))
}

/// Record one finished run into a fresh store; returns the run id.
fn seed_store(db: &Path) -> String {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        use duhem_evidence::{
            EventPayload, EvidenceWriter, SqliteStore, VerdictState, run_started,
        };
        let store = Arc::new(SqliteStore::open(db).await.unwrap());
        let mut w = EvidenceWriter::begin(store, "01SHIP", "ship.yml", BTreeMap::new())
            .await
            .unwrap();
        w.append(run_started("ship.yml", BTreeMap::new()))
            .await
            .unwrap();
        w.append(EventPayload::RunFinished {
            verdict: VerdictState::Pass,
        })
        .await
        .unwrap();
    });
    "01SHIP".to_string()
}

/// One-shot HTTP stub: accept a single request, capture head + body
/// length, answer 200. Returns the captured request head.
fn stub_hub(listener: TcpListener) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        // Read until we have the full head and the declared body.
        let (head, body_start) = loop {
            let n = stream.read(&mut tmp).unwrap();
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break (String::from_utf8_lossy(&buf[..pos]).into_owned(), pos + 4);
            }
        };
        let content_length: usize = head
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().parse().unwrap())
            })
            .unwrap_or(0);
        while buf.len() < body_start + content_length {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .unwrap();
        head
    })
}

#[test]
fn ship_posts_the_bundle_with_hash_and_token() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("duhem.db");
    let run_id = seed_store(&db);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/ingest", listener.local_addr().unwrap());
    let captured = stub_hub(listener);

    let out = Command::new(bin())
        .args(["ship", &run_id, "--db"])
        .arg(&db)
        .args(["--hub-url", &url])
        .env("DUHEM_HUB_TOKEN", "sekret")
        .output()
        .expect("spawn duhem");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("shipped run 01SHIP"), "stdout: {stdout}");

    let head = captured.join().unwrap().to_ascii_lowercase();
    assert!(head.starts_with("post /ingest"), "head: {head}");
    assert!(head.contains("x-duhem-bundle-version: 1"), "head: {head}");
    assert!(head.contains("x-duhem-content-hash: "), "head: {head}");
    assert!(
        head.contains("authorization: bearer sekret"),
        "head: {head}"
    );
    assert!(
        head.contains("content-type: application/json"),
        "head: {head}"
    );
}

#[test]
fn ship_if_configured_no_ops_without_a_hub() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("duhem.db");
    let run_id = seed_store(&db);

    let out = Command::new(bin())
        .args(["ship", &run_id, "--db"])
        .arg(&db)
        .arg("--if-configured")
        .env_remove("DUHEM_HUB_URL")
        .output()
        .expect("spawn duhem");
    assert!(out.status.success(), "no-op skip must exit 0");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("skipping"),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn ship_without_hub_and_without_opt_out_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("duhem.db");
    let run_id = seed_store(&db);

    let out = Command::new(bin())
        .args(["ship", &run_id, "--db"])
        .arg(&db)
        .env_remove("DUHEM_HUB_URL")
        .output()
        .expect("spawn duhem");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("DUHEM_HUB_URL"),
        "stderr should name the remedy"
    );
}
