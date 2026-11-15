//! Real CLI -> real dashboard -> real SQLite; no component substitutes.
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;

use duhem_dashboard::{EvidenceReader, ingest::ingest_router, router};
use duhem_evidence::{RunBundle, SqliteStore, Store, VerdictState};

async fn server(
    db: &Path,
    enabled: bool,
) -> (String, Arc<SqliteStore>, tokio::task::JoinHandle<()>) {
    let store = Arc::new(SqliteStore::open(db).await.unwrap());
    let reader = EvidenceReader::new(Arc::new(SqliteStore::open_read_only(db).await.unwrap()));
    let app = if enabled {
        router(reader).merge(ingest_router(store.clone(), Some("test-token".into())))
    } else {
        router(reader)
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/ingest", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, store, task)
}

fn command(path: &Path, db: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_duhem"));
    command
        .args([
            "run",
            "--no-live",
            "--no-env-file",
            "--reporter",
            "json",
            "--db",
        ])
        .arg(db)
        .arg(path)
        .current_dir(path.parent().unwrap());
    for key in [
        "DUHEM_HUB_URL",
        "DUHEM_HUB_TOKEN",
        "DUHEM_SHARD",
        "DUHEM_RUN_ID",
        "DUHEM_PARENT_RUN_ID",
        "DUHEM_PARENT_DB_PATH",
        "CI_NODE_INDEX",
        "CI_NODE_TOTAL",
        "BUILDKITE_PARALLEL_JOB",
        "BUILDKITE_PARALLEL_JOB_COUNT",
        "CIRCLE_NODE_INDEX",
        "CIRCLE_NODE_TOTAL",
    ] {
        command.env_remove(key);
    }
    command
}

fn leaf(path: &Path, passing: bool) {
    std::fs::write(path, format!("verification: ingest-test\ncriteria:\n  - id: A\n    description: first\n    checks: [{{id: a, assertions: ['{passing}']}}]\n  - id: B\n    description: second\n    checks: [{{id: b, assertions: ['true']}}]\n")).unwrap();
}

fn assert_pass(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("auto-send:"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shards_auto_send_replayed_tree_and_identical_resend_is_a_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let definition = tmp.path().join("leaf.yml");
    leaf(&definition, true);
    let manifest = tmp.path().join("duhem.yml");
    std::fs::write(
        &manifest,
        "manifest_version: 1\nverifications: [{path: leaf.yml}]\n",
    )
    .unwrap();
    let (url, store, server) = server(&tmp.path().join("server.db"), true).await;
    for shard in ["1/2", "2/2"] {
        let db = tmp.path().join(format!("shard-{}.db", &shard[..1]));
        let output = command(&manifest, &db)
            .args(["--shard", shard, "--run-id", "same-invocation"])
            .env("DUHEM_HUB_URL", &url)
            .env("DUHEM_HUB_TOKEN", "test-token")
            .output()
            .unwrap();
        assert_pass(&output);
    }
    let records = store.list_runs().await.unwrap();
    assert_eq!(records.len(), 5, "logical parent + two suites + two leaves");
    let parent = records
        .iter()
        .find(|run| run.lineage.parent_run_id.is_none())
        .unwrap();
    assert_eq!(parent.verdict, None);
    let base = url.strip_suffix("/api/ingest").unwrap();
    let client = reqwest::Client::new();
    let list: serde_json::Value = client
        .get(format!("{base}/api/runs"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["children"].as_array().unwrap().len(), 2);
    for child in list[0]["children"].as_array().unwrap() {
        assert_eq!(child["origin"], "shard");
        assert_eq!(child["children"][0]["verdict"], "pass");
    }
    let detail: serde_json::Value = client
        .get(format!("{base}/api/runs/{}", parent.run_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["children"].as_array().unwrap().len(), 2);
    assert!(detail["verdict"].is_null());
    let failure = client
        .get(format!("{base}/api/runs/{}/failure", parent.run_id))
        .send()
        .await
        .unwrap();
    assert!(failure.status().is_success());
    let source = SqliteStore::open_read_only(tmp.path().join("shard-1.db"))
        .await
        .unwrap();
    let source_records = source.list_runs().await.unwrap();
    let shard = source_records
        .iter()
        .find(|run| run.lineage.origin == Some(duhem_evidence::RunOrigin::Shard))
        .unwrap();
    let standalone = RunBundle::from_store(&source, &shard.run_id).await.unwrap();
    let fresh = SqliteStore::open(tmp.path().join("first-arrival.db"))
        .await
        .unwrap();
    fresh
        .ingest_bundle(&standalone, &standalone.content_hash().unwrap())
        .await
        .unwrap();
    fresh
        .ingest_bundle(&standalone, &standalone.content_hash().unwrap())
        .await
        .unwrap();
    assert_eq!(
        fresh.list_runs().await.unwrap().len(),
        2,
        "first shard creates its logical parent once"
    );
    let leaf = source_records
        .iter()
        .find(|run| run.verdict == Some(VerdictState::Pass))
        .unwrap();
    let mut bundle = RunBundle::from_store(&source, &leaf.run_id).await.unwrap();
    let count = store.run_events(&leaf.run_id).await.unwrap().len();
    let post = |bundle: &RunBundle| {
        client
            .post(&url)
            .bearer_auth("test-token")
            .header("content-type", "application/json")
            .header("x-duhem-bundle-version", "1")
            .header("x-duhem-content-hash", bundle.content_hash().unwrap())
            .body(bundle.wire_bytes().unwrap())
    };
    assert_eq!(post(&bundle).send().await.unwrap().status(), 204);
    assert_eq!(store.list_runs().await.unwrap().len(), 5);
    assert_eq!(store.run_events(&leaf.run_id).await.unwrap().len(), count);
    // A false header verdict cannot override replayed event evidence.
    let mut false_header = bundle.clone();
    false_header.run.run_id = "false-header".into();
    false_header.run.verdict = Some(VerdictState::Fail);
    assert_eq!(post(&false_header).send().await.unwrap().status(), 204);
    assert_eq!(
        store
            .get_run("false-header")
            .await
            .unwrap()
            .unwrap()
            .verdict,
        Some(VerdictState::Pass)
    );
    // The header's summary is not a judgment input. Replay rejects a forged
    // event verdict and the transaction leaves the destination unchanged.
    bundle.run.run_id = "forged".into();
    if let duhem_evidence::EventPayload::RunFinished { verdict, .. } =
        &mut bundle.events.last_mut().unwrap().payload
    {
        *verdict = Some(VerdictState::Fail);
    }
    assert_eq!(post(&bundle).send().await.unwrap().status(), 400);
    assert!(store.get_run("forged").await.unwrap().is_none());
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_run_auto_sends_and_ingest_requires_auth_and_opt_in() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("leaf.yml");
    leaf(&path, true);
    let (url, store, task) = server(&tmp.path().join("enabled.db"), true).await;
    let output = command(&path, &tmp.path().join("local.db"))
        .env("DUHEM_HUB_URL", &url)
        .env("DUHEM_HUB_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_pass(&output);
    let runs = store.list_runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].verdict, Some(VerdictState::Pass));
    let bundle = RunBundle::from_store(store.as_ref(), &runs[0].run_id)
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let request = |url: &str| {
        client
            .post(url)
            .header("content-type", "application/json")
            .header("x-duhem-bundle-version", "1")
            .header("x-duhem-content-hash", bundle.content_hash().unwrap())
            .body(bundle.wire_bytes().unwrap())
    };
    assert_eq!(request(&url).send().await.unwrap().status(), 401);
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("test-token")
            .body("malformed")
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("test-token")
            .body(vec![0; duhem_dashboard::ingest::MAX_BUNDLE_BYTES + 1])
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    let (disabled, disabled_store, disabled_task) =
        server(&tmp.path().join("disabled.db"), false).await;
    assert!(
        !request(&disabled)
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    assert!(disabled_store.list_runs().await.unwrap().is_empty());
    task.abort();
    disabled_task.abort();
}

#[test]
fn unreachable_hub_only_warns_and_preserves_pass_and_fail_exit_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("leaf.yml");
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/ingest", socket.local_addr().unwrap());
    drop(socket);
    for passing in [true, false] {
        leaf(&path, passing);
        let baseline = command(&path, &tmp.path().join(format!("base-{passing}.db")))
            .output()
            .unwrap();
        let sent = command(&path, &tmp.path().join(format!("sent-{passing}.db")))
            .env("DUHEM_HUB_URL", &url)
            .output()
            .unwrap();
        assert_eq!(baseline.status.code(), sent.status.code());
        assert_eq!(sent.status.success(), passing);
        let baseline: serde_json::Value = serde_json::from_slice(&baseline.stdout).unwrap();
        let result: serde_json::Value = serde_json::from_slice(&sent.stdout).unwrap();
        assert_eq!(baseline["verdict"], result["verdict"]);
        assert_eq!(
            String::from_utf8_lossy(&sent.stderr)
                .lines()
                .filter(|line| line.starts_with("warning: auto-send:"))
                .count(),
            1
        );
    }
}
