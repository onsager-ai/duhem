//! Static export (#87): render the JSON API to files, copy the SPA
//! bundle and artifacts, and produce a self-contained directory a
//! plain file host (S3, GH Pages, `python -m http.server`) can serve.
//!
//! Layout under `--out`:
//!
//! ```text
//! index.html, assets/…                  # the SPA bundle
//! api/runs.json                         # GET /api/runs
//! api/runs/<run-id>.json                # GET /api/runs/:id
//! api/runs/<run-id>/trace.jsonl         # wire-format event stream
//! api/runs/<run-id>/checks/<c>::<k>.json
//! run/<run-id>/artifact/<sha>.<ext>     # artifact bytes (#53 path)
//! ```
//!
//! Every URL the export emits is relative to the export root, so the
//! tree works under any base path. Live affordances are omitted: an
//! export is a snapshot, so `live` is forced to `false` everywhere
//! (#84's serve-mode-only boundary).

use std::fs;
use std::path::Path;

use anyhow::Context;

use crate::model::RunsListEntry;
use crate::reader::{EvidenceReader, extension_for, sniff_content_type};
use crate::server::spa_assets;

#[derive(Debug, Default)]
pub struct ExportStats {
    pub runs: usize,
    pub checks: usize,
    pub artifacts: usize,
    pub spa_files: usize,
}

pub async fn export(reader: &EvidenceReader, out: &Path) -> anyhow::Result<ExportStats> {
    let mut stats = ExportStats::default();
    fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;

    for (name, bytes) in spa_assets() {
        write_file(out, &name, &bytes)?;
        stats.spa_files += 1;
    }

    let mut list = reader.list().await?;
    for entry in &mut list {
        freeze_live(entry);
    }
    write_file(out, "api/runs.json", &serde_json::to_vec_pretty(&list)?)?;

    for run_id in leaf_run_ids(&list) {
        export_run(reader, out, &run_id, &mut stats).await?;
    }

    // ② VD-over-time snapshots (#193): one history document per
    // verification name on the list.
    let mut names: Vec<String> = list.iter().map(|e| e.verification.clone()).collect();
    names.sort();
    names.dedup();
    for name in names {
        if let Some(history) = reader.verification_history(&name).await? {
            write_file(
                out,
                &format!("api/verifications/{name}/history.json"),
                &serde_json::to_vec_pretty(&history)?,
            )?;
        }
    }
    Ok(stats)
}

async fn export_run(
    reader: &EvidenceReader,
    out: &Path,
    run_id: &str,
    stats: &mut ExportStats,
) -> anyhow::Result<()> {
    let Some(mut detail) = reader.run_detail(run_id).await? else {
        return Ok(());
    };
    detail.live = false;
    write_file(
        out,
        &format!("api/runs/{run_id}.json"),
        &serde_json::to_vec_pretty(&detail)?,
    )?;

    let trace = reader
        .raw_events_jsonl(run_id)
        .await?
        .context("run listed a moment ago vanished")?;
    write_file(
        out,
        &format!("api/runs/{run_id}/trace.jsonl"),
        trace.as_bytes(),
    )?;
    stats.runs += 1;

    for criterion in &detail.criteria {
        for check in &criterion.checks {
            let Some(mut check_detail) = reader
                .check_detail(run_id, &criterion.id, &check.id)
                .await?
            else {
                continue;
            };
            for artifact in &mut check_detail.artifacts {
                let Some((bytes, mime)) = reader.artifact(run_id, &artifact.id).await? else {
                    continue;
                };
                let ext = extension_for(sniff_content_type(&bytes));
                debug_assert_eq!(mime, sniff_content_type(&bytes));
                let rel = format!("run/{run_id}/artifact/{}.{ext}", artifact.id);
                write_file(out, &rel, &bytes)?;
                // Relative to the export root — the SPA fetches it
                // relative to the document base.
                artifact.url = rel;
                stats.artifacts += 1;
            }
            write_file(
                out,
                &format!(
                    "api/runs/{run_id}/checks/{}::{}.json",
                    criterion.id, check.id
                ),
                &serde_json::to_vec_pretty(&check_detail)?,
            )?;
            stats.checks += 1;
        }
    }
    Ok(())
}

fn leaf_run_ids(list: &[RunsListEntry]) -> Vec<String> {
    let mut ids = Vec::new();
    for entry in list {
        match &entry.children {
            Some(children) => ids.extend(children.iter().map(|c| c.run_id.clone())),
            None => ids.push(entry.run_id.clone()),
        }
    }
    ids
}

fn freeze_live(entry: &mut RunsListEntry) {
    entry.live = false;
    if let Some(children) = &mut entry.children {
        for child in children {
            freeze_live(child);
        }
    }
}

/// Write `rel` under `out`, refusing any path that could escape the
/// export root. Several `rel` segments are evidence-derived strings
/// (run dir names, criterion / check ids from the trace), so a
/// malicious or corrupted trace must not be able to smuggle `..`,
/// an absolute path, or a drive prefix into a write location.
fn write_file(out: &Path, rel: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let rel_path = Path::new(rel);
    let escapes = rel_path
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)));
    if escapes || rel.is_empty() {
        anyhow::bail!("refusing to export path {rel:?}: it would escape the output directory");
    }
    let path = out.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
