//! `consumer-pins` — warn when a drift-monitored consumer's pinned
//! `duhem` version has fallen behind a `[breaking]` CHANGELOG entry
//! (issue #380).
//!
//! This is a separate signal from `drift-chreode.yml`: that job proves
//! a consumer's Verification Definition still *passes* against a
//! freshly-built `duhem`. This one only reads the consumer's pin and
//! warns when it's stale — a consumer can be passing (or, like
//! ostrom-hub, not drift-monitored at all) and still carry a pin that
//! will bite on the next upgrade.
//!
//! ## Registry format
//!
//! Consumers are listed in `.github/drift-consumers.yml`, a YAML
//! sequence of `{ repo, path, pin }` entries: `repo` is the consumer's
//! GitHub repo, `path` is the workflow file (relative to that repo)
//! that pins the `duhem` version, and `pin` is a regex with exactly
//! one capture group extracting the pinned version.
//!
//! Parsed with `serde_yml::Value` — already an xtask dependency — kept
//! deliberately untyped (no `#[derive(Deserialize)]`) so no new crate
//! (`serde` with the `derive` feature isn't one of xtask's direct
//! dependencies today) is needed for three string fields.
//!
//! ## Inputs
//!
//! This subcommand does no networking. The consumer *file contents*
//! (fetched by the caller — see `.github/workflows/consumer-pins.yml`)
//! are passed in with one repeatable flag:
//!
//!     cargo run -p xtask -- consumer-pins \
//!         --consumer onsager-ai/chreode=path/to/duhem.yml \
//!         --consumer onsager-ai/ostrom-hub=path/to/ci.yml
//!
//! A registry entry with no matching `--consumer` flag, a missing
//! file, a non-compiling or non-matching pin regex, or an unparseable
//! pin value is reported as a warning naming the reason — never a
//! silent pass.
//!
//! ## Current version
//!
//! Compared against `duhem_schema::SCHEMA_VERSION` — the same
//! current-version source `schema-drift` already reads (`xtask/src/
//! schema_drift.rs`) — rather than re-parsing the workspace
//! `Cargo.toml`; the release process keeps the two in lockstep (a
//! release commit bumps both).
//!
//! ## Output
//!
//! A GitHub `::warning::` annotation per stale or unreadable consumer,
//! plus a Markdown table appended to `$GITHUB_STEP_SUMMARY` when that
//! env var is set. Always exits `0` — this is an advisory, scheduled
//! signal, never a gate.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;

const REGISTRY_PATH: &str = ".github/drift-consumers.yml";
const CHANGELOG_PATH: &str = "CHANGELOG.md";

pub fn run(args: Vec<String>) -> Result<()> {
    let root = workspace_root()?;
    let consumer_files = parse_consumer_args(&args)?;

    let registry_path = root.join(REGISTRY_PATH);
    let registry_src = std::fs::read_to_string(&registry_path)
        .with_context(|| format!("read {}", registry_path.display()))?;
    let entries =
        parse_registry(&registry_src).with_context(|| format!("parse {REGISTRY_PATH}"))?;

    let changelog_path = root.join(CHANGELOG_PATH);
    let changelog_src = std::fs::read_to_string(&changelog_path)
        .with_context(|| format!("read {}", changelog_path.display()))?;
    let sections = parse_release_sections(&changelog_src);

    let current = parse_version(duhem_schema::SCHEMA_VERSION).ok_or_else(|| {
        anyhow!(
            "current SCHEMA_VERSION `{}` does not parse as a version",
            duhem_schema::SCHEMA_VERSION
        )
    })?;

    let mut stale_rows: Vec<StaleRow> = Vec::new();
    let mut warning_count = 0usize;

    for entry in &entries {
        match evaluate_consumer(entry, &consumer_files, &sections, current) {
            Ok(Outcome::Current(pin)) => {
                eprintln!(
                    "consumer-pins: {} pin v{pin} is current (v{current})",
                    entry.repo
                );
            }
            Ok(Outcome::Stale { pin, breaking }) => {
                warning_count += 1;
                println!(
                    "::warning::consumer-pins: {} pin v{pin} is behind current v{current}; crosses breaking change(s) {}",
                    entry.repo,
                    format_pr_list(&breaking)
                );
                stale_rows.push(StaleRow {
                    repo: entry.repo.clone(),
                    pin,
                    breaking,
                });
            }
            Err(reason) => {
                warning_count += 1;
                println!("::warning::consumer-pins: {}: {reason}", entry.repo);
            }
        }
    }

    write_job_summary(&entries, &stale_rows, current)?;

    eprintln!(
        "consumer-pins: {} consumer(s) checked, {warning_count} warning(s)",
        entries.len()
    );
    Ok(())
}

enum Outcome {
    Current(Version),
    Stale {
        pin: Version,
        breaking: BTreeSet<u64>,
    },
}

struct StaleRow {
    repo: String,
    pin: Version,
    breaking: BTreeSet<u64>,
}

#[derive(Debug)]
struct ConsumerEntry {
    repo: String,
    path: String,
    pin: String,
}

fn evaluate_consumer(
    entry: &ConsumerEntry,
    consumer_files: &HashMap<String, String>,
    sections: &[ReleaseSection],
    current: Version,
) -> Result<Outcome, String> {
    let path = consumer_files
        .get(&entry.repo)
        .ok_or_else(|| format!("no `--consumer {}=<path>` provided", entry.repo))?;

    let contents = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "cannot read consumer file `{path}` (registry path {}): {e}",
            entry.path
        )
    })?;

    let pin = extract_pin_version(&entry.pin, &contents, &entry.path)?;

    if pin >= current {
        return Ok(Outcome::Current(pin));
    }
    let breaking = breaking_entries_in_range(sections, pin, current);
    Ok(Outcome::Stale { pin, breaking })
}

/// Apply `pin_regex` to `contents` and parse capture group 1 as a
/// version. `file_label` is only used to make the error message
/// actionable (it names the registry `path`, not the on-disk temp
/// path the caller fetched it to).
fn extract_pin_version(
    pin_regex: &str,
    contents: &str,
    file_label: &str,
) -> Result<Version, String> {
    let regex = Regex::new(pin_regex)
        .map_err(|e| format!("pin regex `{pin_regex}` does not compile: {e}"))?;
    let captures = regex
        .captures(contents)
        .ok_or_else(|| format!("pin regex `{pin_regex}` found no match in {file_label}"))?;
    let raw = captures
        .get(1)
        .ok_or_else(|| format!("pin regex `{pin_regex}` has no capture group 1"))?
        .as_str();
    parse_version(raw)
        .ok_or_else(|| format!("pin value `{raw}` in {file_label} does not parse as a version"))
}

fn breaking_entries_in_range(
    sections: &[ReleaseSection],
    pin: Version,
    current: Version,
) -> BTreeSet<u64> {
    sections
        .iter()
        .filter(|section| section.version > pin && section.version <= current)
        .flat_map(|section| section.breaking_prs.iter().copied())
        .collect()
}

fn format_pr_list(prs: &BTreeSet<u64>) -> String {
    prs.iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn write_job_summary(
    entries: &[ConsumerEntry],
    stale: &[StaleRow],
    current: Version,
) -> Result<()> {
    let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") else {
        return Ok(());
    };
    if path.is_empty() {
        return Ok(());
    }

    let mut body = String::from("## Consumer pin currency\n\n");
    if stale.is_empty() {
        body.push_str(&format!(
            "All {} consumer pin(s) checked are current with v{current}.\n",
            entries.len()
        ));
    } else {
        body.push_str("| Consumer | Pin | Current | Breaking PRs crossed |\n");
        body.push_str("|---|---|---|---|\n");
        for row in stale {
            body.push_str(&format!(
                "| {} | v{} | v{current} | {} |\n",
                row.repo,
                row.pin,
                format_pr_list(&row.breaking)
            ));
        }
    }

    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open GITHUB_STEP_SUMMARY at {path}"))?;
    file.write_all(body.as_bytes())
        .context("write GITHUB_STEP_SUMMARY")
}

fn parse_consumer_args(args: &[String]) -> Result<HashMap<String, String>> {
    let mut files = HashMap::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let value = if let Some(inline) = arg.strip_prefix("--consumer=") {
            inline.to_string()
        } else if arg == "--consumer" {
            iter.next()
                .ok_or_else(|| anyhow!("--consumer expects a `repo=path` value"))?
                .clone()
        } else {
            bail!("unknown arg: {arg}");
        };
        let (repo, path) = value
            .split_once('=')
            .ok_or_else(|| anyhow!("--consumer value `{value}` must be `repo=path`"))?;
        if repo.is_empty() || path.is_empty() {
            bail!("--consumer value `{value}` must be `repo=path` with both non-empty");
        }
        files.insert(repo.to_string(), path.to_string());
    }
    Ok(files)
}

fn parse_registry(src: &str) -> Result<Vec<ConsumerEntry>> {
    let doc: serde_yml::Value =
        serde_yml::from_str(src).map_err(|e| anyhow!("invalid YAML: {e}"))?;
    let items = doc
        .as_sequence()
        .ok_or_else(|| anyhow!("expected a top-level YAML sequence of consumer entries"))?;

    let mut entries = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        entries.push(ConsumerEntry {
            repo: registry_field(item, "repo", index)?,
            path: registry_field(item, "path", index)?,
            pin: registry_field(item, "pin", index)?,
        });
    }
    Ok(entries)
}

fn registry_field(item: &serde_yml::Value, key: &str, index: usize) -> Result<String> {
    item.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("entry {index}: missing or non-string field `{key}`"))
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_version(raw: &str) -> Option<Version> {
    let mut parts = raw.trim().split('.');
    let major = parse_component(parts.next()?)?;
    let minor = parse_component(parts.next()?)?;
    let patch = match parts.next() {
        Some(part) => parse_component(part)?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some(Version {
        major,
        minor,
        patch,
    })
}

fn parse_component(raw: &str) -> Option<u64> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

struct ReleaseSection {
    version: Version,
    breaking_prs: Vec<u64>,
}

/// Parse every `## vX.Y.Z — ...` release section and the PR numbers
/// named by its `- [breaking]` entries. Deliberately loose compared to
/// `changelog_lint`'s full structural lint — this only needs to
/// recover version boundaries and breaking-entry refs, not validate
/// the ledger.
fn parse_release_sections(src: &str) -> Vec<ReleaseSection> {
    let mut sections = Vec::new();
    let mut current: Option<ReleaseSection> = None;

    for line in src.lines() {
        if let Some(version) = parse_version_heading(line) {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some(ReleaseSection {
                version,
                breaking_prs: Vec::new(),
            });
            continue;
        }
        if line.starts_with("## ") {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            continue;
        }
        if let Some(section) = current.as_mut()
            && let Some(mut prs) = breaking_entry_prs(line)
        {
            section.breaking_prs.append(&mut prs);
        }
    }
    if let Some(section) = current.take() {
        sections.push(section);
    }
    sections
}

fn parse_version_heading(line: &str) -> Option<Version> {
    let rest = line.strip_prefix("## v")?;
    let version_str = rest.split_whitespace().next()?;
    parse_version(version_str)
}

fn breaking_entry_prs(line: &str) -> Option<Vec<u64>> {
    if !line.trim_start().starts_with("- [breaking]") {
        return None;
    }
    Some(extract_pr_numbers(line))
}

fn extract_pr_numbers(line: &str) -> Vec<u64> {
    static PR_REF: OnceLock<Regex> = OnceLock::new();
    let re = PR_REF.get_or_init(|| Regex::new(r"#([0-9]+)").expect("PR ref regex is valid"));
    re.captures_iter(line)
        .filter_map(|caps| caps.get(1)?.as_str().parse::<u64>().ok())
        .collect()
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

    /// A small fixture mirroring the real CHANGELOG's shape, covering
    /// the v0.2.1 → v0.5.1 span the spec issue itself uses as its
    /// worked example.
    const FIXTURE_CHANGELOG: &str = "\
## Unreleased

## v0.5.1 — 2026-09-24
- [clarifying] Results rail links lifecycle detail. (#557)

## v0.5.0 — 2026-09-23
- [breaking] Browser sessions cascade. (#548)
- [breaking] Teardown always runs at every level. (#547)
- [additive] RunSummary carries lifecycle. (#524)

## v0.4.5 — 2026-09-14
- [additive] Bounded for_each now supports check steps. (#521)

## v0.4.0 — 2026-09-04
- [breaking] $pages placeholders fill braces. (#495)
- [additive] step_finished carries a masked detail. (#494)

## v0.3.0 — 2026-08-28
- [breaking] $-leading with: values must parse as expressions. (#461)
- [breaking] Catalog-aware validation rejects unregistered uses. (#430, #431, #432)
- [breaking] validate rejects a bare, unknown, or wrong-arity $runtime helper offline. (#405)
- [clarifying] just preflight gates against fetched origin/main. (#474)

## v0.2.2 — 2026-08-04
- [additive] Reusable flows accept a description. (#387)

## v0.2.1 — 2026-08-04
- [additive] duhem run <leaf-path> resolves $pages.*. (#384)
";

    #[test]
    fn pin_extraction_accepts_the_quoted_env_shape() {
        let pin = extract_pin_version(
            r"DUHEM_VERSION: '([0-9.]+)'",
            "  DUHEM_VERSION: '0.2.1'\n",
            "duhem.yml",
        )
        .expect("pin extracts");
        assert_eq!(
            pin,
            Version {
                major: 0,
                minor: 2,
                patch: 1
            }
        );
    }

    #[test]
    fn pin_extraction_accepts_the_npx_at_shape() {
        let pin = extract_pin_version(
            r"npx duhem@([0-9.]+)",
            "  run: npx duhem@0.4.0 run .duhem\n",
            "ci.yml",
        )
        .expect("pin extracts");
        assert_eq!(
            pin,
            Version {
                major: 0,
                minor: 4,
                patch: 0
            }
        );
    }

    #[test]
    fn unparseable_pin_is_reported_not_silently_passed() {
        let err = extract_pin_version(
            r"DUHEM_VERSION: '([0-9.]+)'",
            "  DUHEM_VERSION: 'latest'\n",
            "duhem.yml",
        )
        .unwrap_err();
        assert!(err.contains("does not parse as a version"), "{err}");
    }

    #[test]
    fn unmatched_pin_regex_is_reported() {
        let err = extract_pin_version(r"DUHEM_VERSION: '([0-9.]+)'", "no pin here\n", "duhem.yml")
            .unwrap_err();
        assert!(err.contains("found no match"), "{err}");
    }

    #[test]
    fn pin_equal_to_current_has_no_breaking_entries() {
        let sections = parse_release_sections(FIXTURE_CHANGELOG);
        let current = Version {
            major: 0,
            minor: 5,
            patch: 1,
        };
        let breaking = breaking_entries_in_range(&sections, current, current);
        assert!(breaking.is_empty());
    }

    #[test]
    fn breaking_range_excludes_entries_at_or_below_the_pin() {
        let sections = parse_release_sections(FIXTURE_CHANGELOG);
        let pin = Version {
            major: 0,
            minor: 4,
            patch: 0,
        };
        let current = Version {
            major: 0,
            minor: 5,
            patch: 1,
        };
        let breaking = breaking_entries_in_range(&sections, pin, current);
        // Only v0.5.0's two breaking entries are strictly above the
        // v0.4.0 pin and at-or-below v0.5.1 current. v0.4.0's own
        // #495 sits AT the pin and must be excluded.
        assert_eq!(
            breaking,
            BTreeSet::from([547, 548]),
            "expected exactly the v0.5.0 breaking PRs"
        );
    }

    #[test]
    fn breaking_range_from_the_issues_worked_example() {
        let sections = parse_release_sections(FIXTURE_CHANGELOG);
        let pin = Version {
            major: 0,
            minor: 2,
            patch: 1,
        };
        let current = Version {
            major: 0,
            minor: 5,
            patch: 1,
        };
        let breaking = breaking_entries_in_range(&sections, pin, current);
        // The spec issue's own worked example: pin 0.2.1, current
        // 0.5.1, seven `[breaking]` entries (#430-432 share one bullet).
        assert_eq!(
            breaking,
            BTreeSet::from([405, 430, 431, 432, 461, 495, 547, 548])
        );
    }

    #[test]
    fn registry_parses_the_two_seeded_consumers() {
        let src = "\
- repo: onsager-ai/chreode
  path: .github/workflows/duhem.yml
  pin: \"DUHEM_VERSION: '([0-9.]+)'\"
- repo: onsager-ai/ostrom-hub
  path: .github/workflows/ci.yml
  pin: \"npx duhem@([0-9.]+)\"
";
        let entries = parse_registry(src).expect("registry parses");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].repo, "onsager-ai/chreode");
        assert_eq!(entries[0].path, ".github/workflows/duhem.yml");
        assert_eq!(entries[0].pin, "DUHEM_VERSION: '([0-9.]+)'");
        assert_eq!(entries[1].repo, "onsager-ai/ostrom-hub");
        assert_eq!(entries[1].pin, "npx duhem@([0-9.]+)");
    }

    #[test]
    fn registry_entry_missing_a_field_is_rejected() {
        let src = "- repo: onsager-ai/chreode\n  path: x\n";
        let err = parse_registry(src).unwrap_err();
        assert!(format!("{err:#}").contains("missing or non-string field `pin`"));
    }

    #[test]
    fn consumer_args_parse_both_flag_shapes() {
        let files = parse_consumer_args(&[
            "--consumer".to_string(),
            "onsager-ai/chreode=/tmp/a.yml".to_string(),
            "--consumer=onsager-ai/ostrom-hub=/tmp/b.yml".to_string(),
        ])
        .expect("args parse");
        assert_eq!(
            files.get("onsager-ai/chreode").map(String::as_str),
            Some("/tmp/a.yml")
        );
        assert_eq!(
            files.get("onsager-ai/ostrom-hub").map(String::as_str),
            Some("/tmp/b.yml")
        );
    }
}
