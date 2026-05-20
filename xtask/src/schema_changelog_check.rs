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

    let changelog_added = added_lines(&root, &base, CHANGELOG_PATH)?;
    let unreleased_lines: Vec<&str> = changelog_added
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|s| s.as_str())
        .collect();

    if !changelog_added_in_unreleased(&root, &base)? {
        eprintln!(
            "schema-changelog-check: schema files touched ({}) but no new lines added to `## Unreleased` in {CHANGELOG_PATH}",
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
        "schema-changelog-check: {} schema file(s) touched, {} new line(s) added to `## Unreleased`",
        touched_schema.len(),
        unreleased_lines.len()
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

/// Lines added (with leading `+` stripped) to a path between `base`
/// and `HEAD`. Header lines (`+++ b/path`) are filtered out.
fn added_lines(root: &Path, base: &str, path: &str) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args([
            "diff",
            "--no-color",
            "--unified=0",
            &format!("{base}...HEAD"),
            "--",
            path,
        ])
        .current_dir(root)
        .output()
        .context("git diff for changelog failed")?;
    if !out.status.success() {
        bail!(
            "git diff {base}...HEAD -- {path} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            if l.starts_with("+++") {
                None
            } else {
                l.strip_prefix('+').map(|rest| rest.to_string())
            }
        })
        .collect())
}

/// Walk the post-diff CHANGELOG line by line; for each `+`-added line,
/// determine whether it falls inside the `## Unreleased` section in
/// the *new* file (i.e. after the diff is applied).
///
/// Approach: read the current `CHANGELOG.md`, mark which line numbers
/// fall under `## Unreleased`, then ask git for `--unified=0`
/// hunk-header line numbers and check overlap.
fn changelog_added_in_unreleased(root: &Path, base: &str) -> Result<bool> {
    let cl_path = root.join(CHANGELOG_PATH);
    let src =
        std::fs::read_to_string(&cl_path).with_context(|| format!("read {}", cl_path.display()))?;
    let unreleased = unreleased_line_range(&src);
    let Some((start, end)) = unreleased else {
        bail!("`## Unreleased` heading missing from {CHANGELOG_PATH}");
    };

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
    let diff = String::from_utf8_lossy(&out.stdout);
    Ok(any_added_line_in_range(&diff, start, end))
}

/// 1-based inclusive `[start, end]` line range covered by the
/// `## Unreleased` heading in `src`. `end` is the line just before the
/// next `## ` heading, or the file's last line.
fn unreleased_line_range(src: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut start: Option<usize> = None;
    let mut end: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if start.is_none() && (trimmed == "## Unreleased" || trimmed == "## [Unreleased]") {
            start = Some(i + 1);
            continue;
        }
        if start.is_some() && line.starts_with("## ") {
            end = Some(i); // last line of the section is `i` (1-based of line before this)
            break;
        }
    }
    let s = start?;
    let e = end.unwrap_or(lines.len());
    Some((s, e))
}

/// Parse `git diff --unified=0` hunk headers and return true when any
/// post-image hunk overlaps `[start, end]` (1-based inclusive).
///
/// Hunk header format: `@@ -<old>,<old_count> +<new>,<new_count> @@`,
/// where `<new_count>` may be omitted (defaults to 1). A hunk with
/// `<new_count> == 0` is a pure deletion — not an addition, so skip.
fn any_added_line_in_range(diff: &str, start: usize, end: usize) -> bool {
    for line in diff.lines() {
        if !line.starts_with("@@") {
            continue;
        }
        // Extract the `+<new>,<new_count>` token.
        let Some(after_plus) = line.split('+').nth(1) else {
            continue;
        };
        let after_plus = after_plus.split_whitespace().next().unwrap_or("");
        let (new_start, new_count) = match after_plus.split_once(',') {
            Some((a, b)) => (a, b),
            None => (after_plus, "1"),
        };
        let new_start: usize = new_start.parse().unwrap_or(0);
        let new_count: usize = new_count.parse().unwrap_or(1);
        if new_count == 0 {
            continue;
        }
        let hunk_end = new_start + new_count - 1;
        if new_start <= end && hunk_end >= start {
            return true;
        }
    }
    false
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
    fn unreleased_range_is_inclusive_to_next_heading() {
        let src = "# CL\n\n## Unreleased\n\n- foo\n- bar\n\n## v0.1.0 — 2026-05-19\n- old\n";
        let (s, e) = unreleased_line_range(src).expect("range");
        assert_eq!(s, 3);
        // Section spans lines 3..=7 (inclusive of the blank line just
        // before the next heading on line 8).
        assert_eq!(e, 7);
    }

    #[test]
    fn unreleased_range_handles_bracketed_heading() {
        let src = "## [Unreleased]\n- entry\n";
        let (s, e) = unreleased_line_range(src).expect("range");
        assert_eq!(s, 1);
        assert_eq!(e, 2);
    }

    #[test]
    fn unreleased_range_missing_returns_none() {
        let src = "# CL\n\n## v0.1.0 — 2026-05-19\n- old\n";
        assert!(unreleased_line_range(src).is_none());
    }

    #[test]
    fn hunk_header_overlap_detects_addition_in_range() {
        // Addition lands at lines 4..=5 of the new file.
        let diff = "@@ -3,0 +4,2 @@\n+- new entry\n+- another\n";
        assert!(any_added_line_in_range(diff, 3, 7));
        assert!(!any_added_line_in_range(diff, 8, 12));
    }

    #[test]
    fn hunk_header_default_count_is_one() {
        // `+4` with no `,count` defaults to 1 line.
        let diff = "@@ -4 +4 @@\n-old\n+new\n";
        assert!(any_added_line_in_range(diff, 4, 4));
        assert!(!any_added_line_in_range(diff, 5, 10));
    }

    #[test]
    fn hunk_header_pure_deletion_doesnt_count_as_add() {
        // `+4,0` means zero lines added at position 4.
        let diff = "@@ -4,2 +4,0 @@\n-gone\n-also-gone\n";
        assert!(!any_added_line_in_range(diff, 1, 100));
    }
}
