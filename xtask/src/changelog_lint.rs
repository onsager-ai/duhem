//! Whole-file `CHANGELOG.md` lint.
//!
//! This complements the diff-scoped schema changelog touch gate. It
//! validates the durable ledger structure and release coverage without
//! reading the archived long-form history.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use regex::Regex;

const CHANGELOG_PATH: &str = "CHANGELOG.md";
const EMPTY_RELEASE_MARKER: &str = "- _No schema-impacting changes._";
const MAX_ENTRY_CHARS: usize = 400;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Violation {
    line: usize,
    message: String,
}

#[derive(Debug)]
struct ReleaseSection {
    version: Version,
    heading_line: usize,
    body_start: usize,
    body_end: usize,
}

#[derive(Debug)]
struct LintReport {
    violations: Vec<Violation>,
    entries: usize,
    release_sections: usize,
    release_tags_checked: usize,
    tag_check_skipped: bool,
}

pub(crate) fn run(root: &Path) -> Result<()> {
    let path = root.join(CHANGELOG_PATH);
    let source =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let release_tags = git_release_tags(root)?;
    let report = lint_changelog(&source, &release_tags);

    if report.tag_check_skipped {
        eprintln!(
            "schema-changelog-check: notice: no v<major>.<minor>.<patch> git tags found; release-section coverage skipped"
        );
    }

    if !report.violations.is_empty() {
        for violation in &report.violations {
            eprintln!("{CHANGELOG_PATH}:{}: {}", violation.line, violation.message);
        }
        bail!(
            "{CHANGELOG_PATH} lint failed with {} violation(s)",
            report.violations.len()
        );
    }

    let tag_summary = if report.tag_check_skipped {
        "release-tag coverage skipped".to_string()
    } else {
        format!("{} release tag(s) checked", report.release_tags_checked)
    };
    eprintln!(
        "schema-changelog-check: {CHANGELOG_PATH} lint passed ({} entries, {} release sections, {tag_summary})",
        report.entries, report.release_sections
    );
    Ok(())
}

fn lint_changelog(source: &str, release_tags: &[Version]) -> LintReport {
    let lines: Vec<&str> = source.lines().collect();
    let mut violations = Vec::new();
    let mut entries = 0;
    let mut headings = Vec::new();
    let mut previous: Option<(Version, usize)> = None;

    for (index, line) in lines.iter().enumerate() {
        let line_number = index + 1;
        if line.starts_with("- [") {
            entries += 1;
            lint_entry(line, line_number, &mut violations);
        }

        if !line.starts_with("## v") {
            continue;
        }

        let parsed_version = parse_heading_version(line);
        if !valid_version_heading(line) {
            violations.push(Violation {
                line: line_number,
                message:
                    "version heading must match `## vX.Y.Z — YYYY-MM-DD` with a valid ISO date"
                        .to_string(),
            });
        }

        let Some(version) = parsed_version else {
            continue;
        };
        if let Some((newer, newer_line)) = previous
            && version >= newer
        {
            violations.push(Violation {
                line: line_number,
                message: format!(
                    "version headings must descend in semver order; {version} is not older than {newer} on line {newer_line}"
                ),
            });
        }
        previous = Some((version, line_number));
        headings.push((version, index));
    }

    let sections = release_sections(&lines, &headings);
    for section in &sections {
        lint_empty_release_section(&lines, section, &mut violations);
    }

    let heading_versions: BTreeSet<Version> =
        sections.iter().map(|section| section.version).collect();
    let tag_check_skipped = release_tags.is_empty();
    if !tag_check_skipped {
        let mut sorted_tags = release_tags.to_vec();
        sorted_tags.sort_unstable();
        sorted_tags.dedup();
        for tag in sorted_tags {
            if !heading_versions.contains(&tag) {
                violations.push(Violation {
                    line: 1,
                    message: format!(
                        "release tag `{tag}` has no corresponding `## {tag} — YYYY-MM-DD` section"
                    ),
                });
            }
        }
    }

    LintReport {
        violations,
        entries,
        release_sections: sections.len(),
        release_tags_checked: release_tags.len(),
        tag_check_skipped,
    }
}

fn lint_entry(line: &str, line_number: usize, violations: &mut Vec<Violation>) {
    let char_count = line.chars().count();
    if char_count > MAX_ENTRY_CHARS {
        violations.push(Violation {
            line: line_number,
            message: format!(
                "entry is {char_count} characters; maximum is {MAX_ENTRY_CHARS} (including tag and refs)"
            ),
        });
    }

    if has_placeholder_ref(line) {
        violations.push(Violation {
            line: line_number,
            message: "placeholder refs are not allowed; replace `#TBD`, `(#)`, `#N`, or `#N/A` with a real `#<number>` ref"
                .to_string(),
        });
    }

    if !entry_regex().is_match(line) {
        violations.push(Violation {
            line: line_number,
            message: "entry must match `- [breaking|additive|clarifying] <text>. (#N)`; multiple refs use `(#212, #213)`"
                .to_string(),
        });
    }
}

fn has_placeholder_ref(line: &str) -> bool {
    line.to_ascii_lowercase().contains("#tbd")
        || line.contains("(#)")
        || line.contains("#N")
        || line.contains("#N/A")
}

fn entry_regex() -> &'static Regex {
    static ENTRY: OnceLock<Regex> = OnceLock::new();
    ENTRY.get_or_init(|| {
        Regex::new(r"^- \[(breaking|additive|clarifying)\] .+\. \(#[0-9]+(?:, #[0-9]+)*\)$")
            .expect("entry regex is valid")
    })
}

fn heading_regex() -> &'static Regex {
    static HEADING: OnceLock<Regex> = OnceLock::new();
    HEADING.get_or_init(|| {
        Regex::new(r"^## v([0-9]+)\.([0-9]+)\.([0-9]+) — ([0-9]{4}-[0-9]{2}-[0-9]{2})$")
            .expect("heading regex is valid")
    })
}

fn valid_version_heading(line: &str) -> bool {
    let Some(captures) = heading_regex().captures(line) else {
        return false;
    };
    parse_numeric_version(
        captures.get(1).expect("major capture").as_str(),
        captures.get(2).expect("minor capture").as_str(),
        captures.get(3).expect("patch capture").as_str(),
    )
    .is_some()
        && NaiveDate::parse_from_str(captures.get(4).expect("date capture").as_str(), "%Y-%m-%d")
            .is_ok()
}

fn parse_heading_version(line: &str) -> Option<Version> {
    let version = line.strip_prefix("## v")?.split_whitespace().next()?;
    parse_version_numbers(version)
}

fn parse_release_tag(tag: &str) -> Option<Version> {
    parse_version_numbers(tag.strip_prefix('v')?)
}

fn parse_version_numbers(version: &str) -> Option<Version> {
    let mut parts = version.split('.');
    let parsed = parse_numeric_version(parts.next()?, parts.next()?, parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(parsed)
}

fn parse_numeric_version(major: &str, minor: &str, patch: &str) -> Option<Version> {
    fn component(value: &str) -> Option<u64> {
        if value.is_empty()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return None;
        }
        value.parse().ok()
    }

    Some(Version {
        major: component(major)?,
        minor: component(minor)?,
        patch: component(patch)?,
    })
}

fn release_sections(lines: &[&str], headings: &[(Version, usize)]) -> Vec<ReleaseSection> {
    headings
        .iter()
        .map(|(version, heading_index)| {
            let body_start = heading_index + 1;
            let body_end = lines[body_start..]
                .iter()
                .position(|line| line.starts_with("## "))
                .map_or(lines.len(), |offset| body_start + offset);
            ReleaseSection {
                version: *version,
                heading_line: heading_index + 1,
                body_start,
                body_end,
            }
        })
        .collect()
}

fn lint_empty_release_section(
    lines: &[&str],
    section: &ReleaseSection,
    violations: &mut Vec<Violation>,
) {
    let body = &lines[section.body_start..section.body_end];
    if body.iter().any(|line| line.starts_with("- [")) {
        return;
    }
    let nonblank: Vec<&str> = body
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();
    if nonblank != [EMPTY_RELEASE_MARKER] {
        violations.push(Violation {
            line: section.heading_line,
            message: format!(
                "empty released section `{}` must contain exactly `{EMPTY_RELEASE_MARKER}`",
                section.version
            ),
        });
    }
}

fn git_release_tags(root: &Path) -> Result<Vec<Version>> {
    let output = Command::new("git")
        .args(["tag", "--list"])
        .current_dir(root)
        .output()
        .context("git tag --list failed")?;
    if !output.status.success() {
        bail!(
            "git tag --list failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| parse_release_tag(line.trim()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    fn messages(source: &str, tags: &[Version]) -> Vec<String> {
        lint_changelog(source, tags)
            .violations
            .into_iter()
            .map(|violation| violation.message)
            .collect()
    }

    fn entry_with_length(length: usize) -> String {
        let prefix = "- [additive] ";
        let suffix = " (#1)";
        let text_length = length - prefix.len() - suffix.len();
        format!("{prefix}{}.{suffix}", "x".repeat(text_length - 1))
    }

    #[test]
    fn entry_shape_accepts_single_and_multiple_refs() {
        let source = "## Unreleased\n\
- [additive] A valid entry. (#212)\n\
- [clarifying] Another valid entry. (#212, #213)\n";
        assert!(messages(source, &[]).is_empty());
    }

    #[test]
    fn entry_shape_rejects_malformed_entry() {
        let source = "## Unreleased\n- [additive] Missing punctuation (#212)\n";
        assert!(messages(source, &[])[0].contains("entry must match"));
    }

    #[test]
    fn placeholder_refs_are_rejected() {
        for entry in [
            "- [additive] Placeholder. (#TBD)",
            "- [additive] Placeholder. (#tbd)",
            "- [additive] Placeholder. (#)",
            "- [additive] Placeholder. (#N)",
            "- [additive] Placeholder. (#N/A)",
        ] {
            let source = format!("## Unreleased\n{entry}\n");
            assert!(
                messages(&source, &[])
                    .iter()
                    .any(|message| message.contains("placeholder refs"))
            );
        }
    }

    #[test]
    fn entry_length_allows_400_and_rejects_401_characters() {
        let valid = format!("## Unreleased\n{}\n", entry_with_length(400));
        assert!(messages(&valid, &[]).is_empty());

        let invalid = format!("## Unreleased\n{}\n", entry_with_length(401));
        assert!(
            messages(&invalid, &[])
                .iter()
                .any(|message| message.contains("401 characters"))
        );
    }

    #[test]
    fn version_heading_requires_a_valid_date() {
        for heading in ["## v0.1.0", "## v0.1.0 — 2026-02-30"] {
            let source = format!("{heading}\n{EMPTY_RELEASE_MARKER}\n");
            assert!(
                messages(&source, &[])
                    .iter()
                    .any(|message| message.contains("valid ISO date"))
            );
        }
    }

    #[test]
    fn version_headings_must_descend() {
        let source = "## v0.1.0 — 2026-01-01\n\
- _No schema-impacting changes._\n\
## v0.1.1 — 2026-01-02\n\
- _No schema-impacting changes._\n";
        assert!(
            messages(source, &[])
                .iter()
                .any(|message| message.contains("must descend"))
        );
    }

    #[test]
    fn c027428_regression_missing_release_heading_for_tag_is_rejected() {
        let source = "## Unreleased\n\
## v0.1.5 — 2026-01-01\n\
- _No schema-impacting changes._\n";
        let violations = messages(source, &[version(0, 1, 5), version(0, 1, 6)]);
        assert!(
            violations
                .iter()
                .any(|message| message.contains("release tag `v0.1.6`"))
        );
    }

    #[test]
    fn no_release_tags_skips_coverage_check() {
        let report = lint_changelog(
            "## v0.1.0 — 2026-01-01\n- _No schema-impacting changes._\n",
            &[],
        );
        assert!(report.violations.is_empty());
        assert!(report.tag_check_skipped);
        assert_eq!(report.release_tags_checked, 0);
    }

    #[test]
    fn empty_released_section_requires_explicit_marker() {
        let source = "## v0.1.0 — 2026-01-01\n\n";
        assert!(
            messages(source, &[])
                .iter()
                .any(|message| message.contains("empty released section"))
        );
    }

    #[test]
    fn empty_unreleased_section_never_requires_marker() {
        let source = "## Unreleased\n\
\n\
## v0.1.0 — 2026-01-01\n\
- _No schema-impacting changes._\n";
        assert!(messages(source, &[]).is_empty());
    }

    #[test]
    fn non_release_tags_are_ignored() {
        for tag in [
            "pr307-pre-rebase-backup",
            "pr2-wip-backup",
            "schema-v0.1.2",
            "v0.1.2-rc1",
        ] {
            assert_eq!(parse_release_tag(tag), None);
        }
        assert_eq!(parse_release_tag("v0.1.2"), Some(version(0, 1, 2)));
    }
}
