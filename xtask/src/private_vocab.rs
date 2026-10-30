//! Shared DENY/allow vocabulary. Keep published-skill and tracker scans identical.

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
