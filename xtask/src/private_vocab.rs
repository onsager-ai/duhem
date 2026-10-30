//! Shared policy definitions. Published-skill process vocabulary and verified
//! private-name signatures are distinct policies with one maintained source.

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
    ("chreode", Match::Substr),
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
    let mut hay = line.to_lowercase();
    for allow in ALLOW_SUBSTRINGS {
        hay = hay.replace(allow, "");
    }
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

/// Private-name policy uses opaque IDs/signatures so the policy source and
/// measurement logs do not republish the identifiers being protected.
/// Visibility verified on 2026-10-30. Remove a signature when its repository
/// becomes public; additions require a fresh visibility check.
pub(crate) struct PrivateName {
    pub id: &'static str,
    pub length: usize,
    pub digest: [u8; 32],
}

pub(crate) const PRIVATE_NAMES: &[PrivateName] = &[
    PrivateName {
        id: "P01",
        length: 7,
        digest: [
            134, 237, 146, 26, 232, 92, 126, 155, 130, 77, 186, 34, 106, 95, 128, 215, 200, 81,
            216, 125, 130, 184, 141, 72, 123, 67, 49, 159, 243, 32, 193, 195,
        ],
    },
    PrivateName {
        id: "P02",
        length: 10,
        digest: [
            106, 26, 13, 243, 225, 68, 240, 161, 44, 37, 0, 240, 255, 73, 123, 80, 31, 253, 224,
            145, 133, 70, 98, 25, 80, 224, 197, 33, 153, 58, 23, 206,
        ],
    },
];

pub(crate) fn scan_private_line<'a>(line: &str, rules: &'a [PrivateName]) -> Vec<&'a str> {
    use sha2::{Digest, Sha256};
    let hay = line.to_ascii_lowercase();
    rules
        .iter()
        .filter(|rule| {
            hay.as_bytes()
                .windows(rule.length)
                .any(|window| Sha256::digest(window)[..] == rule.digest)
        })
        .map(|rule| rule.id)
        .collect()
}

/// Stable opaque IDs allow aggregate per-term measurement without logging names.
pub(crate) fn skill_rule_ids(line: &str) -> Vec<String> {
    scan_line(line)
        .iter()
        .map(|token| {
            let index = DENY
                .iter()
                .position(|(needle, _)| needle == token)
                .expect("known token");
            format!("S{:02}", index + 1)
        })
        .collect()
}

pub(crate) fn all_skill_rule_ids() -> impl Iterator<Item = String> {
    (1..=DENY.len()).map(|index| format!("S{index:02}"))
}
