//! Real CLI -> real dashboard -> real SQLite; no component substitutes.
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;

use duhem_dashboard::{EvidenceReader, ingest::ingest_router, router};
use duhem_evidence::{RunBundle, RunStatus, SqliteStore, Store, VerdictState};

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
        "GITHUB_STEP_SUMMARY",
        "DUHEM_SERVER_URL",
        "DUHEM_SERVER_TOKEN",
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

fn ship_command(db: &Path, url: &str) -> Command {
    let isolated = command(&db.with_extension("yml"), db);
    let mut ship = Command::new(isolated.get_program());
    for (key, value) in isolated.get_envs() {
        match value {
            Some(value) => ship.env(key, value),
            None => ship.env_remove(key),
        };
    }
    ship.args(["ship", "root", "--db"])
        .arg(db)
        .args(["--server-url", url])
        .env("DUHEM_SERVER_TOKEN", "test-token");
    ship
}

async fn stored_run(store: Arc<SqliteStore>, id: &str, parent: Option<&str>, finished: bool) {
    use duhem_evidence::{EventPayload, EvidenceWriter, RunLineage, RunOrigin};
    let lineage = RunLineage {
        parent_run_id: parent.map(str::to_owned),
        origin: parent.map(|_| RunOrigin::Invocation),
    };
    let mut writer = EvidenceWriter::begin_scoped_with_secrets_and_lineage(
        store,
        id,
        "fixture.yml",
        Default::default(),
        Default::default(),
        lineage.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    writer
        .append(duhem_evidence::run_started_with_definition_and_lineage(
            "fixture.yml",
            Default::default(),
            None,
            lineage,
        ))
        .await
        .unwrap();
    if finished {
        writer
            .append(EventPayload::RunFinished {
                body_verdict: None,
                verdict: None,
            })
            .await
            .unwrap();
    }
    writer.finish().await.unwrap();
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
async fn ship_skips_unfinished_children_and_their_finished_descendants() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("local.db");
    let local = Arc::new(SqliteStore::open(&db).await.unwrap());
    stored_run(local.clone(), "root", None, true).await;
    stored_run(local.clone(), "unfinished", Some("root"), false).await;
    stored_run(local.clone(), "blocked", Some("unfinished"), true).await;
    stored_run(local, "sibling", Some("root"), true).await;
    let (url, remote, task) = server(&tmp.path().join("remote.db"), true).await;
    let output = ship_command(&db, &url).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("sent 2 of 4 runs; skipped 1 unfinished; skipped 1 descendants"),
        "{stderr}"
    );
    let runs = remote.list_runs().await.unwrap();
    assert_eq!(runs.len(), 2);
    assert!(remote.get_run("root").await.unwrap().is_some());
    assert!(remote.get_run("sibling").await.unwrap().is_some());
    task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_child_blocks_its_descendants_but_not_later_siblings() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("local.db");
    let local = Arc::new(SqliteStore::open(&db).await.unwrap());
    stored_run(local.clone(), "root", None, true).await;
    stored_run(local.clone(), "rejected", Some("root"), true).await;
    stored_run(local.clone(), "blocked", Some("rejected"), true).await;
    stored_run(local, "sibling", Some("root"), true).await;
    let (url, remote, task) = server(&tmp.path().join("remote.db"), true).await;
    // A different existing run with this id makes real ingest reject the child.
    stored_run(remote.clone(), "rejected", None, true).await;
    let output = ship_command(&db, &url).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "sent 2 of 4 runs; skipped 1 descendants; rejected 1: rejected: server answered"
        ),
        "{stderr}"
    );
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.starts_with("warning: ship:"))
            .count(),
        1
    );
    assert!(remote.get_run("root").await.unwrap().is_some());
    assert!(remote.get_run("sibling").await.unwrap().is_some());
    assert!(remote.get_run("blocked").await.unwrap().is_none());
    task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_send_continues_after_a_rejected_nested_run() {
    let tmp = tempfile::tempdir().unwrap();
    let child = tmp.path().join("child.yml");
    leaf(&child, true);
    let path = tmp.path().join("parent.yml");
    let child_command = |id: &str| {
        format!(
            "[{:?}, run, {:?}, --no-live, --run-id, {id}]",
            env!("CARGO_BIN_EXE_duhem"),
            child
        )
    };
    std::fs::write(&path, format!(
        "verification: parent\ncriteria:\n  - id: A\n    description: nested runs pass\n    checks:\n      - id: a\n        steps:\n          - id: rejected\n            uses: cli/invoke\n            with: {{command: {}}}\n          - id: sibling\n            uses: cli/invoke\n            with: {{command: {}}}\n        assertions: ['$steps.rejected.outputs.exit_code == 0', '$steps.sibling.outputs.exit_code == 0']\n",
        child_command("rejected"), child_command("sibling")
    )).unwrap();
    let (url, remote, task) = server(&tmp.path().join("remote.db"), true).await;
    stored_run(remote.clone(), "rejected", None, true).await;
    let output = command(&path, &tmp.path().join("local.db"))
        .env("DUHEM_SERVER_URL", url.strip_suffix("/api/ingest").unwrap())
        .env("DUHEM_SERVER_TOKEN", "test-token")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["verdict"], "pass");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "warning: auto-send: sent 2 of 3 runs; rejected 1: rejected: server answered"
        ),
        "{stderr}"
    );
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.starts_with("warning: auto-send:"))
            .count(),
        1
    );
    assert!(remote.get_run("sibling").await.unwrap().is_some());
    assert_eq!(remote.list_runs().await.unwrap().len(), 3);
    task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_send_ships_a_nested_engine_error_as_aborted() {
    let tmp = tempfile::tempdir().unwrap();
    let child = tmp.path().join("child.yml");
    std::fs::write(&child, "verification: unfinished\ncriteria:\n  - id: A\n    description: engine error\n    checks:\n      - id: a\n        steps:\n          - id: missing\n            uses: cli/invoke\n            if: failure\n            with: {command: ['true']}\n          - id: error\n            uses: cli/invoke\n            with: {command: [sh, -c, $steps.missing.outputs.stdout]}\n        assertions: ['true']\n").unwrap();
    let path = tmp.path().join("parent.yml");
    std::fs::write(&path, format!("verification: parent\ncriteria:\n  - id: A\n    description: nested engine error is expected\n    checks:\n      - id: a\n        steps:\n          - id: nested\n            uses: cli/invoke\n            with:\n              command: [{:?}, run, {:?}, --no-live]\n        assertions: ['$steps.nested.outputs.exit_code != 0']\n", env!("CARGO_BIN_EXE_duhem"), child)).unwrap();
    let db = tmp.path().join("local.db");
    let (url, remote, task) = server(&tmp.path().join("remote.db"), true).await;
    let output = command(&path, &db)
        .env("DUHEM_SERVER_URL", url.strip_suffix("/api/ingest").unwrap())
        .env("DUHEM_SERVER_TOKEN", "test-token")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("warning: auto-send:"), "{stderr}");
    // #680: the engine error seals the nested run as aborted, so it ships.
    let runs = remote.list_runs().await.unwrap();
    assert_eq!(runs.len(), 2);
    let aborted: Vec<_> = runs
        .iter()
        .filter(|run| run.status == RunStatus::Aborted)
        .collect();
    assert_eq!(aborted.len(), 1);
    assert_eq!(aborted[0].verdict, None);
    let local = SqliteStore::open_read_only(db).await.unwrap();
    assert_eq!(local.list_runs().await.unwrap().len(), 2);
    task.abort();
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
    let baseline = command(&manifest, &tmp.path().join("baseline.db"))
        .output()
        .unwrap();
    assert_pass(&baseline);
    let baseline: serde_json::Value = serde_json::from_slice(&baseline.stdout).unwrap();
    let (url, store, server) = server(&tmp.path().join("server.db"), true).await;
    let parent_id = duhem_evidence::distributed::logical_run_id("same-invocation");
    let link = format!(
        "{}/#/run/{parent_id}",
        url.strip_suffix("/api/ingest").unwrap()
    );
    let summary = tmp.path().join("summary.md");
    std::fs::write(&summary, "Existing summary\n").unwrap();
    for shard in ["1/2", "2/2"] {
        let db = tmp.path().join(format!("shard-{}.db", &shard[..1]));
        let output = command(&manifest, &db)
            .env("DUHEM_SHARD", shard)
            .env("DUHEM_RUN_ID", "same-invocation")
            .env("GITHUB_STEP_SUMMARY", &summary)
            .env("DUHEM_SERVER_URL", &url[..url.len() - "/api/ingest".len()])
            .env("DUHEM_SERVER_TOKEN", "test-token")
            .output()
            .unwrap();
        assert_pass(&output);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_ne!(json["run_id"], parent_id);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<_> = stderr
            .lines()
            .filter(|line| line.starts_with("server:"))
            .collect();
        assert_eq!(lines, [format!("server: {link} (sent)")]);
        let block = std::fs::read_to_string(&summary).unwrap();
        assert!(block.starts_with("Existing summary\n"));
        assert!(block.contains(&format!("Shard: {shard}")), "{block}");
        assert_eq!(
            block
                .matches(&format!("[View run](<{link}>) (sent)"))
                .count(),
            if shard == "1/2" { 1 } else { 2 }
        );
        if shard == "1/2" {
            let parent = store
                .list_runs()
                .await
                .unwrap()
                .into_iter()
                .find(|run| run.lineage.parent_run_id.is_none())
                .unwrap();
            let reader = EvidenceReader::new(Arc::new(
                SqliteStore::open_read_only(tmp.path().join("server.db"))
                    .await
                    .unwrap(),
            ));
            let pending: serde_json::Value = reqwest::get(format!(
                "{}/api/runs/{}",
                url.strip_suffix("/api/ingest").unwrap(),
                parent.run_id
            ))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
            assert_eq!(pending["status"], "pending");
            assert!(pending["verdict"].is_null());
            let detail = reader.run_detail(&parent.run_id).await.unwrap().unwrap();
            assert_eq!(detail.status, duhem_evidence::RunStatus::Pending);
            assert_eq!(detail.verdict, None);
            assert_eq!(detail.shards.as_ref().unwrap().missing.len(), 1);
            assert_eq!(parent.status, duhem_evidence::RunStatus::Pending);
            let expired = reader.with_shard_timeout(std::time::Duration::ZERO);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let expired_url = format!(
                "http://{}/api/runs/{}",
                listener.local_addr().unwrap(),
                parent.run_id
            );
            let app = router(expired.clone());
            let expired_server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let response: serde_json::Value = reqwest::get(format!("{expired_url}/failure"))
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(response["verdict"], "inconclusive:missing_observation");
            assert_eq!(response["missing_shards"][0]["index"], 2);
            expired_server.abort();
            let detail = expired.run_detail(&parent.run_id).await.unwrap().unwrap();
            assert_eq!(detail.status, duhem_evidence::RunStatus::Finished);
            assert_eq!(
                detail.verdict,
                Some(VerdictState::Inconclusive(
                    duhem_judge::InconclusiveCause::MissingObservation
                ))
            );
            let envelope = expired
                .failure_envelope(&parent.run_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(envelope.missing_shards.len(), 1);
            assert_eq!(envelope.missing_shards[0].index, 2);
            assert_eq!(envelope.missing_shards[0].units.len(), 1);
            assert!(envelope.missing_shards[0].units[0].contains('B'));
        }
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
    assert_eq!(detail["verdict"], baseline["verdict"]);
    assert_eq!(detail["status"], "finished");
    assert_eq!(detail["check_totals"]["total"], baseline["totals"]["total"]);
    assert_eq!(detail["shards"]["units"].as_array().unwrap().len(), 2);
    assert_eq!(detail["shards"]["missing"], serde_json::json!([]));
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
            .header("x-duhem-version", env!("CARGO_PKG_VERSION"))
            .header("x-duhem-content-hash", bundle.content_hash().unwrap())
            .body(bundle.wire_bytes().unwrap())
    };
    assert_eq!(post(&standalone).send().await.unwrap().status(), 204);
    for (location, key) in [
        ("top-level", "future_field"),
        ("event payload", "future_field"),
    ] {
        let mut raw: serde_json::Value =
            serde_json::from_slice(&standalone.wire_bytes().unwrap()).unwrap();
        if location == "top-level" {
            raw[key] = serde_json::json!(true);
        } else {
            raw["events"][0]["future_field"] = serde_json::json!({"nested": true});
        }
        let response = client
            .post(&url)
            .bearer_auth("test-token")
            .header("content-type", "application/json")
            .header("x-duhem-bundle-version", "1")
            .header("x-duhem-version", "99.0.0")
            .header("x-duhem-content-hash", standalone.content_hash().unwrap())
            .json(&raw)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{location}");
        let message = response.text().await.unwrap();
        assert!(
            message.contains(
                "this run comes from a newer Duhem (client 99.0.0); upgrade this server ("
            ),
            "{location}: {message}"
        );
        assert!(
            message.ends_with("to at least the client's version"),
            "{message}"
        );
    }
    for kind in ["digest", "total", "duplicate"] {
        let mut conflict = standalone.clone();
        conflict.run.run_id = format!("conflict-{kind}");
        if kind == "digest" {
            conflict.run.manifest_digest = Some("different-digest".into());
        } else if kind == "total" {
            conflict.run.shard.as_mut().unwrap().total = 3;
        }
        for event in &mut conflict.events {
            if let duhem_evidence::EventPayload::ClaimScopeDeclared { scope } = &mut event.payload {
                scope.distributed.manifest_digest = conflict.run.manifest_digest.clone();
                scope.distributed.shard = conflict.run.shard.clone();
            }
        }
        let response = post(&conflict).send().await.unwrap();
        assert_eq!(
            response.status(),
            409,
            "{kind}: {}",
            response.text().await.unwrap()
        );
        assert!(store.get_run(&conflict.run.run_id).await.unwrap().is_none());
    }
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
    let summary = tmp.path().join("summary.md");
    let output = command(&path, &tmp.path().join("local.db"))
        .env("GITHUB_STEP_SUMMARY", &summary)
        .env("DUHEM_SERVER_URL", &url[..url.len() - "/api/ingest".len()])
        .env("DUHEM_SERVER_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_pass(&output);
    let runs = store.list_runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].verdict, Some(VerdictState::Pass));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["run_id"], runs[0].run_id);
    let link = format!(
        "{}/#/run/{}",
        url.strip_suffix("/api/ingest").unwrap(),
        runs[0].run_id
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .any(|line| line == format!("server: {link} (sent)"))
    );
    let block = std::fs::read_to_string(summary).unwrap();
    assert!(block.contains("### Duhem: ingest-test"), "{block}");
    assert!(block.contains("Verdict: pass"), "{block}");
    assert!(
        block.contains("Checks: 2 total, 2 passed, 0 failed, 0 inconclusive"),
        "{block}"
    );
    assert!(
        block.contains(&format!("[View run](<{link}>) (sent)")),
        "{block}"
    );
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
            .env_remove("DUHEM_SERVER_URL")
            .env_remove("DUHEM_SERVER_TOKEN")
            .output()
            .unwrap();
        assert_eq!(baseline.status.code(), sent.status.code());
        assert_eq!(sent.status.success(), passing);
        let baseline: serde_json::Value = serde_json::from_slice(&baseline.stdout).unwrap();
        let result: serde_json::Value = serde_json::from_slice(&sent.stdout).unwrap();
        assert_eq!(baseline["verdict"], result["verdict"]);
        let link = format!(
            "{}/#/run/{}",
            url.strip_suffix("/api/ingest").unwrap(),
            result["run_id"].as_str().unwrap()
        );
        assert!(
            String::from_utf8_lossy(&sent.stderr)
                .lines()
                .any(|line| line.starts_with(&format!("server: {link} (upload failed: ")))
        );
        assert_eq!(
            String::from_utf8_lossy(&sent.stderr)
                .lines()
                .filter(|line| line.starts_with("warning: auto-send:"))
                .count(),
            1
        );
    }
}

#[test]
fn no_server_leaves_job_summary_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("leaf.yml");
    leaf(&path, true);
    let summary = tmp.path().join("summary.md");
    for existing in [false, true] {
        if existing {
            std::fs::write(&summary, "Previous job step\n").unwrap();
        }
        let output = command(&path, &tmp.path().join(format!("local-{existing}.db")))
            .env("GITHUB_STEP_SUMMARY", &summary)
            .output()
            .unwrap();
        assert_pass(&output);
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
        assert!(
            !String::from_utf8_lossy(&output.stderr)
                .lines()
                .any(|line| line.starts_with("server:"))
        );
        if existing {
            assert_eq!(
                std::fs::read_to_string(&summary).unwrap(),
                "Previous job step\n"
            );
        } else {
            assert!(!summary.exists());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manifest_legacy_link_names_parent_and_summary_failure_only_warns() {
    let tmp = tempfile::tempdir().unwrap();
    leaf(&tmp.path().join("leaf.yml"), true);
    let manifest = tmp.path().join("duhem.yml");
    std::fs::write(
        tmp.path().join("input.xml"),
        include_str!("../../duhem-runtime/tests/fixtures/junit/pass.xml"),
    )
    .unwrap();
    std::fs::write(&manifest, "manifest_version: 1\nnative:\n  - id: smoke\n    command: [sh, -c, 'cat input.xml > report.xml']\n    report: {format: junit, path: report.xml}\nverifications: [{path: leaf.yml}]\n").unwrap();
    let (url, store, task) = server(&tmp.path().join("remote.db"), true).await;
    let summary = tmp.path().join("summary.md");
    for bad_summary in [false, true] {
        let output = command(
            &manifest,
            &tmp.path().join(format!("local-{bad_summary}.db")),
        )
        .env("DUHEM_HUB_URL", &url)
        .env("DUHEM_HUB_TOKEN", "test-token")
        .env(
            "GITHUB_STEP_SUMMARY",
            if bad_summary { tmp.path() } else { &summary },
        )
        .output()
        .unwrap();
        assert_pass(&output);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let id = json["run_id"].as_str().unwrap();
        assert!(
            store
                .get_run(id)
                .await
                .unwrap()
                .unwrap()
                .lineage
                .parent_run_id
                .is_none()
        );
        let link = format!("{}/#/run/{id}", url.strip_suffix("/api/ingest").unwrap());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr
                .lines()
                .any(|line| line == format!("server: {link} (sent)")),
            "{stderr}"
        );
        if bad_summary {
            assert!(stderr.contains("warning: could not append GitHub job summary:"));
        } else {
            let block = std::fs::read_to_string(&summary).unwrap();
            assert!(block.contains("duhem.yml"), "{block}");
            assert!(
                block.contains("Native runs: 1 total, 1 passed, 0 failed, 0 inconclusive"),
                "{block}"
            );
            assert!(
                block.contains("Native tests: 1 passed, 0 failed, 0 errored, 1 skipped"),
                "{block}"
            );
            assert!(block.contains(&link), "{block}");
        }
    }
    task.abort();
}

#[test]
fn unknown_legacy_endpoint_does_not_invent_dashboard_link() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("leaf.yml");
    leaf(&path, true);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/custom-ingest", listener.local_addr().unwrap());
    drop(listener);
    let summary = tmp.path().join("summary.md");
    let output = command(&path, &tmp.path().join("local.db"))
        .env("DUHEM_HUB_URL", url)
        .env("GITHUB_STEP_SUMMARY", &summary)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("warning: auto-send:"));
    assert!(!stderr.lines().any(|line| line.starts_with("server:")));
    assert!(!summary.exists());
}

#[test]
fn rejected_upload_warns_with_server_reason() {
    use std::io::{BufRead, BufReader, Read, Write};
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("leaf.yml");
    leaf(&path, true);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    // One-request server that rejects the bundle with a reason.
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let reason = "invalid run identity";
        write!(
            reader.get_mut(),
            "HTTP/1.1 400 Bad Request\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reason}",
            reason.len()
        )
        .unwrap();
    });
    let output = command(&path, &tmp.path().join("duhem.db"))
        .env("DUHEM_SERVER_URL", &base)
        .env_remove("DUHEM_HUB_URL")
        .env_remove("DUHEM_SERVER_TOKEN")
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("warning: auto-send: sent 0 of 1 runs; rejected 1:"),
        "{stderr}"
    );
    assert!(
        stderr.contains("server answered 400 Bad Request: invalid run identity"),
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aggregate_preserves_failures_native_gates_and_empty_shards() {
    let tmp = tempfile::tempdir().unwrap();
    let definition = tmp.path().join("leaf.yml");
    let manifest = tmp.path().join("duhem.yml");
    std::fs::write(&manifest, "manifest_version: 1\nnative:\n  - {id: required-native, command: [sh, -c, 'exit 1']}\nverifications: [{path: leaf.yml}]\n").unwrap();
    leaf(&definition, false);
    let baseline = command(&manifest, &tmp.path().join("baseline.db"))
        .output()
        .unwrap();
    let baseline: serde_json::Value = serde_json::from_slice(&baseline.stdout).unwrap();
    let (url, store, task) = server(&tmp.path().join("server.db"), true).await;
    // More machines than units also exercises completed empty shard containers.
    for index in 1..=4 {
        let output = command(&manifest, &tmp.path().join(format!("shard-{index}.db")))
            .args([
                "--shard",
                &format!("{index}/4"),
                "--run-id",
                "failure-and-empty",
            ])
            .env("DUHEM_HUB_URL", &url)
            .env("DUHEM_HUB_TOKEN", "test-token")
            .env_remove("DUHEM_SERVER_URL")
            .env_remove("DUHEM_SERVER_TOKEN")
            .output()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("auto-send:"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let parent = store
        .list_runs()
        .await
        .unwrap()
        .into_iter()
        .find(|run| run.lineage.parent_run_id.is_none())
        .unwrap();
    let reader = EvidenceReader::new(Arc::new(
        SqliteStore::open_read_only(tmp.path().join("server.db"))
            .await
            .unwrap(),
    ));
    let detail = reader.run_detail(&parent.run_id).await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(detail.verdict).unwrap(),
        baseline["verdict"]
    );
    assert_eq!(detail.verdict, Some(VerdictState::Fail));
    assert_eq!(detail.check_totals.unwrap().total, 2);
    assert_eq!(detail.shards.unwrap().arrived.len(), 4);
    assert_eq!(
        detail.layers.iter().map(|layer| layer.tests).sum::<u64>(),
        3
    );
    let failure = reader
        .failure_envelope(&parent.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failure.failing.len(), 1);
    let check = &failure.failing[0];
    assert_eq!(check.criterion_id, "A");
    assert_eq!(check.check_id, "a");
    assert_eq!(check.verdict, Some(VerdictState::Fail));
    assert_eq!(check.assertions.len(), 1);
    assert_eq!(check.assertions[0].state, VerdictState::Fail);
    // The same assertion-only leaf must remain visible when requested directly.
    let leaf_failure = reader
        .failure_envelope(check.run_id.as_deref().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(leaf_failure.failing.len(), 1);
    assert_eq!(
        leaf_failure.failing[0].assertions[0].state,
        VerdictState::Fail
    );
    assert_eq!(failure.failing_native.len(), 1);
    assert!(failure.missing_shards.is_empty());
    task.abort();
}
