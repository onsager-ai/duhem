//! Shared policy definitions. Published-skill process vocabulary and verified
//! private names are distinct policies with one maintained source.

use anyhow::{Context, Result, bail};

/// Substrings stripped from each line *before* scanning, so a legitimate
/// public reference to the open-source project isn't flagged by the
/// bare-word `onsager` rule.
const ALLOW_SUBSTRINGS: &[&str] = &["onsager-ai/duhem-docs", "onsager-ai/duhem"];

#[derive(Clone, Copy)]
enum Match {
    /// Match anywhere (distinctive tokens with no benign substring use).
    Substr,
    /// Match only as a whole word (bounded by non-word chars), so
    /// e.g. `hub` does not fire inside `GitHub`.
    Word,
}

/// Internal-only vocabulary that must never reach a published skill.
const DENY: &[(&str, Match)] = &[
    ("dogfood", Match::Substr),
    ("crawlab", Match::Substr),
    ("asymmetric", Match::Substr),
    ("codeowners", Match::Substr),
    ("onsager-dogfood", Match::Substr),
    ("duhem-dev-process", Match::Substr),
    ("pr-lifecycle", Match::Substr),
    ("pre-push", Match::Substr),
    ("onsager", Match::Word),
    ("seam", Match::Word),
    ("hub", Match::Word),
];

/// Return every denied token found on a line, after stripping the
/// allow-listed substrings. Case-insensitive.
pub(crate) fn scan_line(line: &str) -> Vec<String> {
    let hay = allowed_line(line);
    let mut hits: Vec<String> = Vec::new();
    for (needle, kind) in DENY {
        let found = match kind {
            Match::Substr => hay.contains(needle),
            Match::Word => contains_word(&hay, needle),
        };
        if found {
            hits.push((*needle).to_string());
        }
    }
    hits
}

fn allowed_line(line: &str) -> String {
    let mut hay = line.to_lowercase();
    for allow in ALLOW_SUBSTRINGS {
        hay = hay.replace(allow, "");
    }
    hay
}

/// Whole-word (bounded by non-`[a-z0-9_]`) case-insensitive match.
/// `hay` is already lowercased; `needle` is lowercase.
fn contains_word(hay: &str, needle: &str) -> bool {
    let bytes = hay.as_bytes();
    let nlen = needle.len();
    let mut start = 0;
    while let Some(pos) = hay[start..].find(needle) {
        let at = start + pos;
        let before_ok = at == 0 || !is_word_byte(bytes[at - 1]);
        let after = at + nlen;
        let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        start = at + 1;
    }
    false
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Private policy is supplied at runtime, never embedded in public source.
pub(crate) fn load_private_names() -> Result<Vec<String>> {
    let raw = std::env::var("DUHEM_PRIVATE_NAMES").ok();
    parse_private_names(raw.as_deref())
}

fn parse_private_names(raw: Option<&str>) -> Result<Vec<String>> {
    let raw =
        raw.context("DUHEM_PRIVATE_NAMES must be set to a comma-separated private-name list")?;
    let mut names = Vec::new();
    for name in raw.split(',') {
        let name = name.trim().to_lowercase();
        if name.is_empty() {
            bail!("DUHEM_PRIVATE_NAMES must contain only non-empty comma-separated names");
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Ok(names)
}

/// Only rule positions leave the matcher; callers cannot accidentally log terms.
pub(crate) fn scan_private_line(line: &str, rules: &[&str]) -> Vec<usize> {
    let hay = allowed_line(line);
    rules
        .iter()
        .enumerate()
        .filter_map(|(id, rule)| hay.contains(rule).then_some(id))
        .collect()
}

pub(crate) fn all_skill_terms() -> impl Iterator<Item = &'static str> {
    DENY.iter().map(|(term, _)| *term)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_empty_policy_fails_without_echoing_values() {
        for input in [
            None,
            Some(""),
            Some("  "),
            Some("restricted-example,"),
            Some(",restricted-example"),
        ] {
            let error = parse_private_names(input).unwrap_err().to_string();
            assert!(error.contains("DUHEM_PRIVATE_NAMES"));
            assert!(!error.contains("restricted-example"));
        }
    }

    #[test]
    fn normalizes_and_deduplicates_names_and_matches_without_text() {
        let names = parse_private_names(Some(
            " Restricted-Example , hidden-sample,RESTRICTED-EXAMPLE ",
        ))
        .unwrap();
        assert_eq!(names, ["restricted-example", "hidden-sample"]);
        let rules = names.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(
            scan_private_line("RESTRICTED-EXAMPLE and hidden-sample", &rules),
            [0, 1]
        );
        assert!(scan_private_line("ordinary public content", &rules).is_empty());
    }

    #[test]
    fn public_project_slugs_stay_allowed() {
        assert!(scan_private_line("https://github.com/onsager-ai/duhem", &["onsager"]).is_empty());
        assert!(scan_private_line("onsager-ai/duhem-docs", &["onsager"]).is_empty());
        assert_eq!(
            scan_private_line("onsager outside the slug", &["onsager"]),
            [0]
        );
    }
}
