//! `duhem dashboard` integration tests (#87).
//!
//! The fast tests stub the child with a shell script via
//! `DUHEM_DASHBOARD_BIN` so they assert the CLI's own behavior
//! (flag forwarding, exit-code propagation, the missing-binary
//! message) without needing the dashboard binary built. The real
//! end-to-end (boot the actual server, hit `/api/runs`) is
//! `#[ignore]`d and runs in CI's dashboard lane after
//! `cargo build -p duhem-dashboard`.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn duhem_bin() -> &'static str {
    env!("CARGO_BIN_EXE_duhem")
}

/// Install a fake `duhem-dashboard` that records its argv and exits
/// with `exit_code`.
fn fake_dashboard(dir: &Path, exit_code: i32) -> (PathBuf, PathBuf) {
    let argv_log = dir.join("argv.txt");
    let bin = dir.join("fake-dashboard.sh");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\nexit {exit_code}\n",
        argv_log.display()
    );
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, argv_log)
}

#[test]
fn serve_mode_forwards_flags_and_propagates_exit_code() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, argv_log) = fake_dashboard(tmp.path(), 0);

    let status = Command::new(duhem_bin())
        .env("DUHEM_DASHBOARD_BIN", &bin)
        .args([
            "dashboard",
            "--db",
            "ev.db",
            "--port",
            "8123",
            "--host",
            "0.0.0.0",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let argv = std::fs::read_to_string(argv_log).unwrap();
    assert_eq!(
        argv.lines().collect::<Vec<_>>(),
        vec!["--db", "ev.db", "--port", "8123", "--host", "0.0.0.0"]
    );
}

#[test]
fn export_mode_forwards_the_subcommand() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, argv_log) = fake_dashboard(tmp.path(), 0);

    let status = Command::new(duhem_bin())
        .env("DUHEM_DASHBOARD_BIN", &bin)
        .args(["dashboard", "export", "--out", "site", "--db", "ev.db"])
        .status()
        .unwrap();
    assert!(status.success());
    let argv = std::fs::read_to_string(argv_log).unwrap();
    assert_eq!(
        argv.lines().collect::<Vec<_>>(),
        vec!["--db", "ev.db", "export", "--out", "site"]
    );
}

#[test]
fn child_failure_exit_code_is_propagated() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, _) = fake_dashboard(tmp.path(), 3);

    let status = Command::new(duhem_bin())
        .env("DUHEM_DASHBOARD_BIN", &bin)
        .args(["dashboard"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(3));
}

#[test]
fn missing_binary_fails_with_guidance() {
    let tmp = tempfile::tempdir().unwrap();
    let output = Command::new(duhem_bin())
        .env("DUHEM_DASHBOARD_BIN", tmp.path().join("does-not-exist"))
        .args(["dashboard"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot launch"), "stderr: {stderr}");
    assert!(stderr.contains("DUHEM_DASHBOARD_BIN"), "stderr: {stderr}");
}

/// Real end-to-end: `duhem dashboard` boots the actual server and
/// `/api/runs` answers. Needs `cargo build -p duhem-dashboard` first
/// (CI dashboard lane); resolution uses the target-dir sibling of the
/// `duhem` test binary's target dir.
#[test]
#[ignore = "needs the duhem-dashboard binary built (CI dashboard lane / just dashboard test)"]
fn dashboard_serve_end_to_end() {
    // CARGO_BIN_EXE_duhem lives in target/debug/deps-adjacent layout;
    // the dashboard binary lands in the same target/debug dir.
    let dashboard_bin = Path::new(duhem_bin())
        .parent()
        .unwrap()
        .join("duhem-dashboard");
    assert!(
        dashboard_bin.is_file(),
        "build duhem-dashboard first: cargo build -p duhem-dashboard"
    );

    let evidence = tempfile::tempdir().unwrap();
    // The dashboard opens the store read-only, so an (empty, migrated)
    // store must exist first.
    let db_path = evidence.path().join("duhem.db");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            duhem_evidence::SqliteStore::open(&db_path).await.unwrap();
        });
    // Own process group so teardown can kill the `duhem` wrapper AND
    // the `duhem-dashboard` grandchild it spawned — killing only the
    // wrapper would orphan the server (and leak the stdout pipe).
    use std::os::unix::process::CommandExt;
    let mut child = Command::new(duhem_bin())
        .env("DUHEM_DASHBOARD_BIN", &dashboard_bin)
        .args(["dashboard", "--port", "0", "--db"])
        .arg(&db_path)
        .stdout(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();

    // The server prints "duhem dashboard listening on http://ADDR/ ..."
    let stdout = child.stdout.take().unwrap();
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).unwrap();
    let addr = line
        .split("http://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_else(|| panic!("no listen line: {line}"))
        .to_string();

    let mut stream = std::net::TcpStream::connect(&addr).unwrap();
    write!(
        stream,
        "GET /api/runs HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    // Negative pid = the whole process group (wrapper + server). The
    // `--` keeps `kill` from reading the negative pid as a flag.
    let _ = Command::new("kill")
        .args(["-TERM", "--", &format!("-{}", child.id())])
        .status();
    let _ = child.wait();

    assert!(response.starts_with("HTTP/1.1 200"), "got: {response}");
    assert!(
        response.ends_with("[]"),
        "empty store lists no runs: {response}"
    );
}
