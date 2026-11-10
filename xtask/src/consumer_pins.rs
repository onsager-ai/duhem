//! Consumer-owned pin currency: compare an explicit pin with Duhem's release ledger.
//! No registry, consumer repository access, token or network requests.
use anyhow::{Context, Result, bail};
use regex::Regex;
use std::{collections::BTreeSet, path::Path, sync::OnceLock};

pub fn run(args: Vec<String>) -> Result<()> {
    if args == ["--help"] {
        println!(
            "consumer-pins --pinned <version>\nAdvisory: reports released breaking changes crossed by your pin; does not execute verification definitions."
        );
        return Ok(());
    }
    let pin = parse_args(&args)?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")?;
    let changelog =
        std::fs::read_to_string(root.join("CHANGELOG.md")).context("read Duhem CHANGELOG")?;
    let sections = parse_release_sections(&changelog);
    if sections.is_empty() {
        bail!("Duhem CHANGELOG has no release sections");
    }
    let current =
        parse_version(duhem_schema::SCHEMA_VERSION).context("parse current schema version")?;
    let breaking = breaking_entries_in_range(&sections, pin, current);
    if pin >= current {
        println!("consumer-pins: pin v{pin} is current or newer (v{current})");
    } else {
        println!("{}", stale_annotation(pin, current, &breaking));
    }
    if let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY")
        && !path.is_empty()
    {
        use std::io::Write as _;
        let mut summary = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .context("open GitHub job summary")?;
        writeln!(summary, "## Consumer pin currency\n\nPin: v{pin}; current: v{current}; released breaking PRs crossed: {}.\n", breaking_display(&breaking))
            .context("write GitHub job summary")?;
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Version> {
    let raw = match args {
        [flag, value] if flag == "--pinned" => value.as_str(),
        [inline] if inline.starts_with("--pinned=") => &inline["--pinned=".len()..],
        _ => bail!("consumer-pins requires --pinned <version>"),
    };
    parse_version(raw.strip_prefix('v').unwrap_or(raw))
        .context("--pinned must be a numeric major.minor[.patch] version")
}

fn stale_annotation(pin: Version, current: Version, breaking: &BTreeSet<u64>) -> String {
    let message = if breaking.is_empty() {
        format!(
            "::notice::consumer-pins: pin v{pin} is behind current v{current}, no breaking change crossed"
        )
    } else {
        format!(
            "::warning::consumer-pins: pin v{pin} is behind current v{current}; crosses breaking change(s) {}",
            format_pr_list(breaking)
        )
    };
    escape_annotation(&message)
}

fn escape_annotation(message: &str) -> String {
    message
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
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

fn breaking_display(breaking: &BTreeSet<u64>) -> String {
    if breaking.is_empty() {
        "none".to_string()
    } else {
        format_pr_list(breaking)
    }
}

fn format_pr_list(prs: &BTreeSet<u64>) -> String {
    prs.iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ")
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
    fn stale_annotation_is_a_notice_when_no_breaking_change_crossed() {
        let pin = Version {
            major: 0,
            minor: 5,
            patch: 0,
        };
        let current = Version {
            major: 0,
            minor: 5,
            patch: 1,
        };
        let annotation = stale_annotation(pin, current, &BTreeSet::new());
        assert!(annotation.starts_with("::notice::"), "{annotation}");
        assert!(
            annotation.contains("no breaking change crossed"),
            "{annotation}"
        );
    }

    #[test]
    fn stale_annotation_is_a_warning_when_a_breaking_change_is_crossed() {
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
        let breaking = BTreeSet::from([547, 548]);
        let annotation = stale_annotation(pin, current, &breaking);
        assert!(annotation.starts_with("::warning::"), "{annotation}");
        assert!(
            annotation.contains("crosses breaking change(s) #547, #548"),
            "{annotation}"
        );
    }

    #[test]
    fn breaking_display_shows_none_for_an_empty_set() {
        assert_eq!(breaking_display(&BTreeSet::new()), "none");
        assert_eq!(breaking_display(&BTreeSet::from([547])), "#547");
    }

    #[test]
    fn escape_annotation_neutralizes_an_embedded_forged_annotation() {
        // A consumer-derived value (e.g. a captured pin, or a path
        // quoted in an error) containing a real newline must not let
        // a forged second command through as its own line.
        let raw = "boom\n::error::forged";
        let escaped = escape_annotation(raw);
        assert_eq!(escaped.lines().count(), 1, "{escaped}");
        assert!(!escaped.contains('\n'), "{escaped}");
        assert!(escaped.contains("%0A::error::forged"), "{escaped}");
    }

    #[test]
    fn escape_annotation_round_trips_percent() {
        assert_eq!(escape_annotation("100%"), "100%25");
    }

    #[test]
    fn escape_annotation_does_not_double_escape_its_own_sequences() {
        // Escaping `%` before `\r`/`\n` means the `%` introduced by
        // escaping a real CR/LF is never itself re-escaped to `%25`.
        assert_eq!(escape_annotation("\r"), "%0D");
        assert_eq!(escape_annotation("\n"), "%0A");
    }

    #[test]
    fn explicit_pin_accepts_both_flag_shapes_and_rejects_missing_or_invalid_values() {
        for args in [vec!["--pinned", "0.2.1"], vec!["--pinned=v0.2.1"]] {
            assert_eq!(
                parse_args(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).unwrap(),
                parse_version("0.2.1").unwrap()
            );
        }
        for args in [
            vec![],
            vec!["--pinned"],
            vec!["--pinned", "latest"],
            vec!["--pinned="],
            vec!["--list"],
            vec!["--pinned", "1."],
        ] {
            assert!(parse_args(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
        }
    }

    #[test]
    fn breaking_range_excludes_unreleased_and_releases_above_current() {
        let source = "## Unreleased\n- [breaking] future. (#999)\n## v0.6.0 — 2026-10-01\n- [breaking] later. (#600)\n## v0.5.0 — 2026-09-01\n- [breaking] included. (#500)\n";
        assert_eq!(
            breaking_entries_in_range(
                &parse_release_sections(source),
                parse_version("0.4.0").unwrap(),
                parse_version("0.5.0").unwrap()
            ),
            BTreeSet::from([500])
        );
    }
}
