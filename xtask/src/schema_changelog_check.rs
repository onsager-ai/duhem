//! Changelog-touch check — fails CI if a branch changes
//! `crates/duhem-schema/src/**` or `crates/duhem-evidence/src/**`
//! without adding lines to `CHANGELOG.md`'s `## Unreleased` section.
//!
//! Escape hatch: setting `DUHEM_CHANGELOG_CLARIFYING=1` (typically by
//! the PR-level workflow when the PR body carries an explicit
//! `clarifying` annotation) bypasses the touch requirement. The
//! escape hatch exists because trivial internal renames inside the
//! schema crate are not user-visible schema events; without it,
//! every refactor would drag the schema gate.
//!
//! ## Comparison base
//!
//! Diffs against `origin/main` by default. CI overrides with the
//! `GITHUB_BASE_REF` env var so a PR targeting a non-main branch
//! still diffs against the right base.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

/// Files whose changes are considered schema-impacting. Matched as
/// path prefixes against `git diff --name-only` output. Evidence
/// events surface on the verification wire (criteria → evidence
/// trace → judge), so changes there are also schema-impacting.
const SCHEMA_PATHS: &[&str] = &["crates/duhem-schema/src/", "crates/duhem-evidence/src/"];

const CHANGELOG_PATH: &str = "CHANGELOG.md";
const ESCAPE_ENV: &str = "DUHEM_CHANGELOG_CLARIFYING";

pub fn run(_args: Vec<String>) -> Result<()> {
    let root = workspace_root()?;

    if std::env::var(ESCAPE_ENV).is_ok_and(|v| !v.is_empty() && v != "0") {
        eprintln!(
            "schema-changelog-check: skipped via {ESCAPE_ENV} (PR self-declared as clarifying)"
        );
        return Ok(());
    }

    let base = comparison_base();
    let changed = changed_files(&root, &base)?;
    let touched_schema: Vec<&String> = changed
        .iter()
        .filter(|p| SCHEMA_PATHS.iter().any(|prefix| p.starts_with(prefix)))
        .collect();

    if touched_schema.is_empty() {
        eprintln!(
            "schema-changelog-check: no schema-impacting files changed vs {base}; nothing to check"
        );
        return Ok(());
    }

    let cl_path = root.join(CHANGELOG_PATH);
    let src =
        std::fs::read_to_string(&cl_path).with_context(|| format!("read {}", cl_path.display()))?;
    let Some((unreleased_start, unreleased_end)) = unreleased_line_range(&src) else {
        bail!("`## Unreleased` heading missing from {CHANGELOG_PATH}");
    };

    let cl_diff = changelog_diff(&root, &base)?;
    let added = added_lines_with_positions(&cl_diff);
    let entries: Vec<&AddedLine> = added
        .iter()
        .filter(|a| a.line >= unreleased_start && a.line <= unreleased_end)
        .filter(|a| is_entry_line(&a.content))
        .collect();

    if entries.is_empty() {
        eprintln!(
            "schema-changelog-check: schema files touched ({}) but no new entries added to `## Unreleased` in {CHANGELOG_PATH}",
            touched_schema.len()
        );
        for p in &touched_schema {
            eprintln!("  - touched: {p}");
        }
        eprintln!("\nTo fix:");
        eprintln!(
            "  - append a `- [breaking|additive|clarifying] one-line summary. (#N)` entry to `## Unreleased`, OR"
        );
        eprintln!(
            "  - if the change is genuinely clarifying-only, set `{ESCAPE_ENV}=1` (CI does this when the PR body carries an explicit `clarifying` annotation)."
        );
        bail!("schema change without CHANGELOG entry");
    }

    eprintln!(
        "schema-changelog-check: {} schema file(s) touched, {} new entry line(s) added to `## Unreleased`",
        touched_schema.len(),
        entries.len()
    );
    Ok(())
}

fn comparison_base() -> String {
    if let Ok(base) = std::env::var("GITHUB_BASE_REF")
        && !base.is_empty()
    {
        return format!("origin/{base}");
    }
    "origin/main".to_string()
}

fn changed_files(root: &Path, base: &str) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["diff", "--name-only", &format!("{base}...HEAD")])
        .current_dir(root)
        .output()
        .context("git diff --name-only failed")?;
    if !out.status.success() {
        bail!(
            "git diff --name-only {base}...HEAD failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// Raw `git diff --unified=0 base...HEAD -- CHANGELOG.md` output. The
/// caller parses positions out of it.
fn changelog_diff(root: &Path, base: &str) -> Result<String> {
    let out = Command::new("git")
        .args([
            "diff",
            "--no-color",
            "--unified=0",
            &format!("{base}...HEAD"),
            "--",
            CHANGELOG_PATH,
        ])
        .current_dir(root)
        .output()
        .context("git diff for changelog failed")?;
    if !out.status.success() {
        bail!(
            "git diff {base}...HEAD -- {CHANGELOG_PATH} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[derive(Debug)]
struct AddedLine {
    /// 1-based line number in the post-image (i.e. the file as it
    /// stands at HEAD).
    line: usize,
    content: String,
}

/// Parse `git diff --unified=0` output and return every `+` line with
/// the post-image line number it occupies. Skips diff header lines
/// (`+++ b/path`) and hunk-header `@@` lines.
fn added_lines_with_positions(diff: &str) -> Vec<AddedLine> {
    let mut out: Vec<AddedLine> = Vec::new();
    // Tracks the next post-image line number an `+`-prefixed body
    // line will occupy. Reset by each `@@ -... +N,M @@` header.
    let mut next_new_line: usize = 0;
    for line in diff.lines() {
        if line.starts_with("@@") {
            if let Some((start, _count)) = parse_hunk_new_range(line) {
                next_new_line = start;
            }
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if let Some(body) = line.strip_prefix('+') {
            out.push(AddedLine {
                line: next_new_line,
                content: body.to_string(),
            });
            next_new_line += 1;
        }
        // `-` and context lines don't advance the post-image counter
        // under `--unified=0` (no context lines emitted; deletions
        // come in their own hunks).
    }
    out
}

/// Parse the post-image `(start, count)` from a `@@ -... +S,C @@`
/// header. `count` defaults to 1 when omitted.
fn parse_hunk_new_range(header: &str) -> Option<(usize, usize)> {
    let after_plus = header.split('+').nth(1)?;
    let after_plus = after_plus.split_whitespace().next()?;
    let (s, c) = match after_plus.split_once(',') {
        Some((a, b)) => (a, b),
        None => (after_plus, "1"),
    };
    Some((s.parse().ok()?, c.parse().ok()?))
}

/// Does this added line look like a real CHANGELOG entry? The gate
/// cares about content additions, not whitespace or heading edits.
/// Required shape per the policy: `- [breaking|additive|clarifying]
/// ... (#N)`. We accept the looser `- [` prefix so reflows of an
/// existing entry don't trip a strict-form regex.
fn is_entry_line(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with("- [")
}

/// 1-based inclusive `[start, end]` line range covered by the
/// *body* of the `## Unreleased` section in `src`. `start` is the
/// first line **after** the heading — not the heading itself —
/// so a touch to the heading line alone (e.g. an editor reflow that
/// rewrites it byte-for-byte but registers as a diff) doesn't count
/// as adding an entry. `end` is the line just before the next `## `
/// heading, or the file's last line.
fn unreleased_line_range(src: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut start: Option<usize> = None;
    let mut end: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if start.is_none() && (trimmed == "## Unreleased" || trimmed == "## [Unreleased]") {
            // `i` is 0-based; the heading itself is line `i + 1` and
            // we want the line *after* the heading, so `i + 2`.
            start = Some(i + 2);
            continue;
        }
        if start.is_some() && line.starts_with("## ") {
            end = Some(i); // last line of the section is `i` (1-based of line before this)
            break;
        }
    }
    let s = start?;
    let e = end.unwrap_or(lines.len());
    if s > e { None } else { Some((s, e)) }
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .context("CARGO_MANIFEST_DIR not set; run via `cargo run -p xtask`")?;
    Ok(Path::new(&manifest)
        .parent()
        .ok_or_else(|| anyhow!("xtask manifest has no parent"))?
        .to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreleased_range_skips_the_heading_line() {
        let src = "# CL\n\n## Unreleased\n\n- foo\n- bar\n\n## v0.1.0 — 2026-05-19\n- old\n";
        let (s, e) = unreleased_line_range(src).expect("range");
        // Heading is line 3; the body starts at line 4 (the blank line
        // right under the heading). `e` is the line just before the
        // next `## ` heading on line 8.
        assert_eq!(s, 4);
        assert_eq!(e, 7);
    }

    #[test]
    fn unreleased_range_handles_bracketed_heading() {
        let src = "## [Unreleased]\n- entry\n";
        let (s, e) = unreleased_line_range(src).expect("range");
        // Heading on line 1; body line is 2.
        assert_eq!(s, 2);
        assert_eq!(e, 2);
    }

    #[test]
    fn unreleased_range_with_empty_body_returns_none() {
        // A `## Unreleased` heading immediately followed by the next
        // version heading has no body lines to track. `unreleased_line_range`
        // returns `None` so the gate falls through to "no entries
        // added" and fails as intended.
        let src = "## Unreleased\n## v0.1.0 — 2026-05-19\n- old\n";
        assert!(unreleased_line_range(src).is_none());
    }

    #[test]
    fn unreleased_range_missing_returns_none() {
        let src = "# CL\n\n## v0.1.0 — 2026-05-19\n- old\n";
        assert!(unreleased_line_range(src).is_none());
    }

    #[test]
    fn added_lines_track_post_image_positions() {
        // Two lines added at post-image lines 4 and 5.
        let diff = "@@ -3,0 +4,2 @@\n+- new entry\n+- another\n";
        let added = added_lines_with_positions(diff);
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].line, 4);
        assert_eq!(added[0].content, "- new entry");
        assert_eq!(added[1].line, 5);
        assert_eq!(added[1].content, "- another");
    }

    #[test]
    fn added_lines_default_hunk_count_is_one() {
        // `+4` with no `,count` is a single-line modification.
        let diff = "@@ -4 +4 @@\n-old\n+new\n";
        let added = added_lines_with_positions(diff);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].line, 4);
        assert_eq!(added[0].content, "new");
    }

    #[test]
    fn added_lines_pure_deletion_yields_nothing() {
        let diff = "@@ -4,2 +4,0 @@\n-gone\n-also-gone\n";
        assert!(added_lines_with_positions(diff).is_empty());
    }

    #[test]
    fn entry_shape_filter_accepts_real_entries_and_rejects_noise() {
        assert!(is_entry_line("- [additive] new field. (#42)"));
        assert!(is_entry_line("  - [breaking] renamed. (#7)"));
        assert!(!is_entry_line(""));
        assert!(!is_entry_line("- some prose line"));
        assert!(!is_entry_line("## Unreleased"));
        assert!(!is_entry_line("blank line addition"));
    }

    #[test]
    fn heading_only_touch_does_not_satisfy_the_gate() {
        // Mock: schema file is touched, and the only addition in the
        // changelog is the `## Unreleased` heading itself (e.g. a
        // reflow that registers as a diff). The position-based filter
        // skips the heading line (start is the line *after* the
        // heading), so the gate correctly reports zero entries.
        let cl = "# CL\n\n## Unreleased\n- existing entry\n";
        let (start, end) = unreleased_line_range(cl).expect("range");
        let diff = "@@ -3,0 +3,1 @@\n+## Unreleased\n";
        let added = added_lines_with_positions(diff);
        let in_range: Vec<&AddedLine> = added
            .iter()
            .filter(|a| a.line >= start && a.line <= end && is_entry_line(&a.content))
            .collect();
        assert!(in_range.is_empty(), "heading edit should not count");
    }

    #[test]
    fn real_entry_addition_satisfies_the_gate() {
        let cl = "# CL\n\n## Unreleased\n- existing entry\n";
        let (start, end) = unreleased_line_range(cl).expect("range");
        let diff = "@@ -4,0 +4,1 @@\n+- [additive] new thing. (#9)\n";
        let added = added_lines_with_positions(diff);
        let in_range: Vec<&AddedLine> = added
            .iter()
            .filter(|a| a.line >= start && a.line <= end && is_entry_line(&a.content))
            .collect();
        assert_eq!(in_range.len(), 1);
    }
}
