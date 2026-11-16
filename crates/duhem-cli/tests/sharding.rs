//! Distributed selection exercises the real CLI and real native subprocesses.
#![cfg(unix)]
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

const ENV_KEYS: &[&str] = &[
    "DUHEM_BRANCH",
    "DUHEM_PR",
    "DUHEM_CI_URL",
    "DUHEM_SHARD",
    "DUHEM_RUN_ID",
    "CI_NODE_INDEX",
    "CI_NODE_TOTAL",
    "CI_PIPELINE_ID",
    "CI_JOB_NAME",
    "BUILDKITE_PARALLEL_JOB",
    "BUILDKITE_PARALLEL_JOB_COUNT",
    "BUILDKITE_BUILD_ID",
    "BUILDKITE_STEP_ID",
    "CIRCLE_NODE_INDEX",
    "CIRCLE_NODE_TOTAL",
    "CIRCLE_WORKFLOW_ID",
    "CIRCLE_JOB",
    "GITHUB_RUN_ID",
    "GITHUB_RUN_ATTEMPT",
    "GITHUB_JOB",
    "DUHEM_PARENT_RUN_ID",
];
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
    for key in ENV_KEYS {
        command.env_remove(key);
    }
    command
}
fn summary(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn leaf(root: &Path) -> std::path::PathBuf {
    let path = root.join("leaf.yml");
    std::fs::write(
        &path,
        r#"verification: shard-test
setup:
  - id: leaf-setup
    uses: cli/invoke
    with: {command: [sh, -c, 'echo setup >> leaf-setup'], cwd: .}
native:
  - {id: native-a, command: [sh, -c, 'echo a >> native-a']}
  - {id: native-b, command: [sh, -c, 'echo b >> native-b']}
criteria:
  - id: A
    description: first indivisible criterion
    checks:
      - {id: a1, assertions: ['true']}
      - {id: a2, assertions: ['true']}
  - id: B
    description: second criterion
    checks:
      - {id: b1, assertions: ['true']}
"#,
    )
    .unwrap();
    path
}

#[test]
fn every_ci_entry_generic_and_flag_precedence_work_in_subprocesses() {
    let tmp = tempfile::tempdir().unwrap();
    let path = leaf(tmp.path());
    let cases: &[(&[(&str, &str)], &str)] = &[
        (
            &[
                ("CI_NODE_INDEX", "1"),
                ("CI_NODE_TOTAL", "2"),
                ("CI_PIPELINE_ID", "42"),
                ("CI_JOB_NAME", "test 1/2"),
            ],
            "gitlab",
        ),
        (
            &[
                ("BUILDKITE_PARALLEL_JOB", "0"),
                ("BUILDKITE_PARALLEL_JOB_COUNT", "2"),
                ("BUILDKITE_BUILD_ID", "42"),
                ("BUILDKITE_STEP_ID", "test"),
            ],
            "buildkite",
        ),
        (
            &[
                ("CIRCLE_NODE_INDEX", "0"),
                ("CIRCLE_NODE_TOTAL", "2"),
                ("CIRCLE_WORKFLOW_ID", "42"),
                ("CIRCLE_JOB", "test"),
            ],
            "circleci",
        ),
        (
            &[
                ("DUHEM_SHARD", "1/2"),
                ("GITHUB_RUN_ID", "42"),
                ("GITHUB_RUN_ATTEMPT", "1"),
                ("GITHUB_JOB", "test"),
            ],
            "env",
        ),
    ];
    for (index, (vars, source)) in cases.iter().enumerate() {
        let mut cmd = command(&path, &tmp.path().join(format!("ci-{index}.db")));
        for (key, value) in *vars {
            cmd.env(key, value);
        }
        let value = summary(cmd.output().unwrap());
        assert_eq!(
            value["shard"],
            serde_json::json!({"index":1,"total":2,"source":source})
        );
        assert_eq!(value["origin"], "shard");
        assert_eq!(value["expected_partition"]["total_units"], 4);
        assert_eq!(value["manifest_digest"].as_str().unwrap().len(), 64);

        let mut generic = command(&path, &tmp.path().join(format!("generic-{index}.db")));
        for (key, value) in *vars {
            generic.env(key, value);
        }
        generic
            .env("DUHEM_SHARD", "2/2")
            .env("DUHEM_RUN_ID", "generic");
        let generic = summary(generic.output().unwrap());
        assert_eq!(generic["shard"]["index"], 2);
        assert_eq!(generic["shard"]["source"], "env");

        let mut explicit = command(&path, &tmp.path().join(format!("flag-{index}.db")));
        for (key, value) in *vars {
            explicit.env(key, value);
        }
        explicit
            .env("DUHEM_SHARD", "bad")
            .env("DUHEM_RUN_ID", "generic")
            .args(["--shard", "1/2", "--run-id", "flags"]);
        let explicit = summary(explicit.output().unwrap());
        assert_eq!(explicit["shard"]["source"], "flag");
        assert_ne!(explicit["parent_run_id"], generic["parent_run_id"]);
    }
}

#[test]
fn invalid_shard_stops_before_subprocess_or_database_creation() {
    for value in ["2/1", "1/0", "nope", "0/2", "1/2/3"] {
        let tmp = tempfile::tempdir().unwrap();
        let path = leaf(tmp.path());
        let db = tmp.path().join("invalid.db");
        let output = command(&path, &db)
            .args(["--shard", value, "--run-id", "shared"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("invalid shard"));
        assert!(!db.exists());
        assert!(!tmp.path().join("native-a").exists());
    }
}

#[tokio::test]
async fn two_shards_cover_manifest_once_keep_criteria_whole_and_persist_plan() {
    let tmp = tempfile::tempdir().unwrap();
    leaf(tmp.path());
    let manifest = tmp.path().join("duhem.yml");
    std::fs::write(&manifest, "manifest_version: 1\nnative:\n  - {id: suite, command: [sh, -c, 'echo suite >> suite']}\nverifications: [{path: leaf.yml}]\n").unwrap();
    let mut units = BTreeSet::new();
    let mut criteria = BTreeSet::new();
    let mut native = BTreeSet::new();
    let mut parent = None;
    let mut digest = None;
    let mut shard_ids = BTreeSet::new();
    for k in 1..=2 {
        let db = tmp.path().join(format!("shard-{k}.db"));
        let value = summary(
            command(&manifest, &db)
                .env("DUHEM_BRANCH", "feature/ci-context")
                .env("DUHEM_PR", "42")
                .env("DUHEM_CI_URL", "https://ci.example/run/123")
                .args([
                    "--shard",
                    &format!("{k}/2"),
                    "--run-id",
                    "shared invocation",
                ])
                .output()
                .unwrap(),
        );
        assert_eq!(value["origin"], "shard");
        assert_eq!(value["shard"]["index"], k);
        assert_eq!(value["expected_partition"]["total_units"], 5);
        assert!(
            value["expected_partition"]["excluded"]
                .as_array()
                .unwrap()
                .iter()
                .all(|unit| unit["reason"] == "shard")
        );
        for unit in value["expected_partition"]["units"].as_array().unwrap() {
            assert!(
                units.insert(unit.as_str().unwrap().to_owned()),
                "duplicate unit"
            );
        }
        if let Some(previous) = &parent {
            assert_eq!(&value["parent_run_id"], previous);
        }
        parent = Some(value["parent_run_id"].clone());
        if let Some(previous) = &digest {
            assert_eq!(&value["manifest_digest"], previous);
        }
        digest = Some(value["manifest_digest"].clone());
        assert!(shard_ids.insert(value["run_id"].as_str().unwrap().to_owned()));
        for run in value["runs"].as_array().unwrap() {
            assert_eq!(run["shard"], value["shard"]);
            assert_eq!(run["manifest_digest"], value["manifest_digest"]);
            let trace = duhem_evidence::Trace::from_store(
                &duhem_evidence::SqliteStore::open(&db).await.unwrap(),
                run["run_id"].as_str().unwrap(),
            )
            .await
            .unwrap();
            duhem_evidence::replay(&trace).unwrap();
            for criterion in run["criteria"].as_array().unwrap() {
                assert!(criteria.insert(criterion["id"].as_str().unwrap().to_owned()));
            }
            let checks = &run["claim_scope"]["checks"];
            let owns_a = run["criteria"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == "A");
            assert_eq!(checks["executed"], if owns_a { 2 } else { 1 });
            assert!(
                run["claim_scope"]["excluded"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|entry| entry["reason"] == "shard")
            );
        }
        for run in value["native"].as_array().unwrap() {
            assert!(native.insert(run["id"].as_str().unwrap().to_owned()));
        }
        let store = duhem_evidence::SqliteStore::open(&db).await.unwrap();
        use duhem_evidence::Store;
        let record = store
            .get_run(value["run_id"].as_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.lineage.origin,
            Some(duhem_evidence::RunOrigin::Shard)
        );
        assert_eq!(
            record.lineage.parent_run_id.as_deref(),
            value["parent_run_id"].as_str()
        );
        let logical_parent = store
            .get_run(value["parent_run_id"].as_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        for scope in [&record.scope, &logical_parent.scope] {
            assert_eq!(scope.branch.as_deref(), Some("feature/ci-context"));
            assert_eq!(scope.pr.as_deref(), Some("42"));
            assert_eq!(scope.ci_url.as_deref(), Some("https://ci.example/run/123"));
        }
        let events = store
            .run_events(value["run_id"].as_str().unwrap())
            .await
            .unwrap();
        let plan = duhem_evidence::claim_scope::claim_scope(&events).unwrap();
        assert_eq!(
            plan.distributed.manifest_digest.as_deref(),
            value["manifest_digest"].as_str()
        );
    }
    assert_eq!(units.len(), 5);
    assert_eq!(criteria, BTreeSet::from(["A".into(), "B".into()]));
    assert_eq!(
        native,
        BTreeSet::from(["suite".into(), "native-a".into(), "native-b".into()])
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("leaf-setup"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    for marker in ["suite", "native-a", "native-b"] {
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(marker))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn no_shard_preserves_summary_and_dry_run_and_filter_precedes_partition() {
    let tmp = tempfile::tempdir().unwrap();
    let path = leaf(tmp.path());
    let db = tmp.path().join("ordinary.db");
    let value = summary(command(&path, &db).output().unwrap());
    for key in [
        "shard",
        "origin",
        "parent_run_id",
        "manifest_digest",
        "expected_partition",
    ] {
        assert!(value.get(key).is_none(), "{key}");
        assert!(value["claim_scope"].get(key).is_none(), "{key}");
    }
    let dry = command(&path, &db)
        .args(["--dry-run", "--filter", "A"])
        .output()
        .unwrap();
    assert!(dry.status.success());
    assert_eq!(
        String::from_utf8(dry.stdout).unwrap(),
        "WOULD RUN: A::a1\nWOULD RUN: A::a2\nEXCLUDED NATIVE: native-a, native-b\nRESOLVED INPUT: (none)\n"
    );
    for k in 1..=2 {
        let output = command(&path, &tmp.path().join(format!("filtered-{k}.db")))
            .args([
                "--filter",
                "A::a2",
                "--shard",
                &format!("{k}/2"),
                "--run-id",
                "filtered",
            ])
            .output()
            .unwrap();
        let value = summary(output);
        assert_eq!(value["expected_partition"]["total_units"], 1);
        assert_eq!(value["totals"]["total"], if k == 1 { 1 } else { 0 });
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("leaf-setup"))
                .unwrap()
                .lines()
                .count(),
            1 + k.min(1)
        );
        assert!(value["native"].is_null());
        let store = duhem_evidence::SqliteStore::open(tmp.path().join(format!("filtered-{k}.db")))
            .await
            .unwrap();
        let trace = duhem_evidence::Trace::from_store(&store, value["run_id"].as_str().unwrap())
            .await
            .unwrap();
        duhem_evidence::replay(&trace).unwrap();
    }
}

#[test]
fn empty_manifest_shard_has_a_plan_and_single_leaf_shards_share_local_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let path = leaf(tmp.path());
    let manifest = tmp.path().join("duhem.yml");
    std::fs::write(
        &manifest,
        "manifest_version: 1\nverifications: [{path: leaf.yml}]\n",
    )
    .unwrap();
    let value = summary(
        command(&manifest, &tmp.path().join("empty.db"))
            .args(["--shard", "7/7", "--run-id", "empty"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["runs"], serde_json::json!([]));
    assert_eq!(value["expected_partition"]["units"], serde_json::json!([]));
    assert_eq!(value["expected_partition"]["total_units"], 4);
    assert_eq!(value["shard"]["index"], 7);
    assert!(!tmp.path().join("leaf-setup").exists());
    let db = tmp.path().join("shared.db");
    let first = summary(
        command(&path, &db)
            .args(["--shard", "1/2", "--run-id", "same"])
            .output()
            .unwrap(),
    );
    let second = summary(
        command(&path, &db)
            .args(["--shard", "2/2", "--run-id", "same"])
            .output()
            .unwrap(),
    );
    assert_ne!(first["run_id"], second["run_id"]);
    assert_eq!(first["parent_run_id"], second["parent_run_id"]);
}
