//! #87 static-export tests: the output is self-contained, mirrors the
//! API shapes, and every emitted URL is relative to the export root.

mod common;

use duhem_dashboard::{EvidenceReader, export};
use serde_json::Value;

#[test]
fn export_produces_a_self_contained_tree() {
    let evidence = tempfile::tempdir().unwrap();
    let sha = common::write_passing_run(
        &evidence.path().join("01J0000000000000000000000A"),
        "verifications/create-workspace.yml",
    );
    common::write_failing_run(
        &evidence.path().join("login/01J0000000000000000000000B"),
        "verifications/login/duhem.yml",
    );

    let out = tempfile::tempdir().unwrap();
    let stats = export(&EvidenceReader::new(evidence.path()), out.path()).unwrap();
    assert_eq!(stats.runs, 2);
    assert_eq!(stats.checks, 2);
    assert_eq!(stats.artifacts, 1);
    assert!(stats.spa_files >= 1, "SPA bundle (or placeholder) copied");

    // The SPA entry point.
    assert!(out.path().join("index.html").is_file());

    // Runs list snapshot, with the run-set nesting intact and live
    // affordances frozen off (#84: export is a snapshot).
    let list: Value =
        serde_json::from_slice(&std::fs::read(out.path().join("api/runs.json")).unwrap()).unwrap();
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    fn assert_not_live(row: &Value) {
        assert_eq!(row["live"], false);
        if let Some(children) = row["children"].as_array() {
            children.iter().for_each(assert_not_live);
        }
    }
    rows.iter().for_each(assert_not_live);

    // Per-run snapshots + raw trace passthrough.
    for run_id in ["01J0000000000000000000000A", "01J0000000000000000000000B"] {
        assert!(out.path().join(format!("api/runs/{run_id}.json")).is_file());
        assert!(
            out.path()
                .join(format!("api/runs/{run_id}/trace.jsonl"))
                .is_file()
        );
        assert!(
            out.path()
                .join(format!("api/runs/{run_id}/checks/AC-1::AC-1.1.json"))
                .is_file()
        );
    }

    // The artifact landed at the #53-decided path with a sniffed
    // extension, and the check JSON points at it relatively.
    let artifact_rel = format!("run/01J0000000000000000000000A/artifact/{sha}.png");
    assert_eq!(
        std::fs::read(out.path().join(&artifact_rel)).unwrap(),
        common::png_bytes()
    );
    let check: Value = serde_json::from_slice(
        &std::fs::read(
            out.path()
                .join("api/runs/01J0000000000000000000000A/checks/AC-1::AC-1.1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(check["artifacts"][0]["url"], artifact_rel);
}

/// #87 Test bullet: no absolute URLs anywhere in the exported JSON —
/// the tree must work under any base path.
#[test]
fn exported_json_contains_no_absolute_urls() {
    let evidence = tempfile::tempdir().unwrap();
    common::write_passing_run(
        &evidence.path().join("01J0000000000000000000000A"),
        "verifications/x.yml",
    );
    let out = tempfile::tempdir().unwrap();
    export(&EvidenceReader::new(evidence.path()), out.path()).unwrap();

    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|e| e == "json") {
                let json: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                check_urls(&json, &path, hits);
            }
        }
    }
    fn check_urls(v: &Value, path: &std::path::Path, hits: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                if let Some(url) = map.get("url").and_then(|u| u.as_str())
                    && (url.starts_with('/') || url.contains("://"))
                {
                    hits.push(format!("{}: {url}", path.display()));
                }
                map.values().for_each(|v| check_urls(v, path, hits));
            }
            Value::Array(items) => items.iter().for_each(|v| check_urls(v, path, hits)),
            _ => {}
        }
    }

    let mut hits = Vec::new();
    walk(&out.path().join("api"), &mut hits);
    assert!(hits.is_empty(), "absolute URLs in export: {hits:#?}");
}
