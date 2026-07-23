//! Skill-scrub gate — the firewall for user-facing skills.
//!
//! The authoring skill we publish to product repos (under
//! `templates/product-repo/.claude/skills/`) is a *derived, sanitized*
//! sibling of Duhem's internal dev skills. It must not carry the
//! internal dev context those skills legitimately hold — the dogfood
//! framing, the specific dogfood customers, the seam / trust-model
//! internals, our CODEOWNERS/hub gating, or the internal dev-skill
//! names. Publishing that vocabulary would leak how Duhem is *built* to
//! someone who only wants to *use* it.
//!
//! This lint reads every `.md` under the published-skill tree and fails
//! if any internal token appears. It is a hard gate, not advisory: a
//! leak is a leak. The public repo-slug `onsager-ai/duhem` (and the
//! docs repo) is allow-listed so a legitimate link to the open-source
//! project doesn't trip the bare-word `onsager` rule.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

/// Root of the published-skill tree, relative to the workspace root.
const PUBLISHED_SKILLS_DIR: &str = "templates/product-repo/.claude/skills";

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

struct Violation {
    file: PathBuf,
    line: usize,
    token: String,
}

pub fn run(_args: Vec<String>) -> Result<()> {
    let root = workspace_root()?;
    let base = root.join(PUBLISHED_SKILLS_DIR);

    let files = if base.exists() {
        markdown_files(&base)
    } else {
        Vec::new()
    };

    let mut violations: Vec<Violation> = Vec::new();
    for file in &files {
        let src =
            std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
        let rel = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
        for (idx, line) in src.lines().enumerate() {
            for token in scan_line(line) {
                violations.push(Violation {
                    file: rel.clone(),
                    line: idx + 1,
                    token,
                });
            }
        }
    }

    if !violations.is_empty() {
        eprintln!("skill-scrub violations — internal vocabulary in a published skill:");
        for v in &violations {
            eprintln!("  {}:{}: `{}`", v.file.display(), v.line, v.token);
        }
        eprintln!(
            "\nPublished skills under {PUBLISHED_SKILLS_DIR} are user-facing and must not carry\ninternal dev context. Cut the token, or generalize it (e.g. \"your repo's CI\")."
        );
        bail!(
            "{} internal-token leak(s) in published skill(s)",
            violations.len()
        );
    }

    eprintln!(
        "skill-scrub: {} published skill file(s) clean of internal vocabulary",
        files.len()
    );
    Ok(())
}

/// Return every denied token found on a line, after stripping the
/// allow-listed substrings. Case-insensitive.
fn scan_line(line: &str) -> Vec<String> {
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

fn markdown_files(base: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(base, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
            out.push(path);
        }
    }
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
    fn clean_line_passes() {
        assert!(scan_line("Author a Verification Definition for your product.").is_empty());
        assert!(scan_line("Self-gate the suite in your product's own CI.").is_empty());
    }

    #[test]
    fn internal_tokens_are_caught() {
        assert_eq!(
            scan_line("This is the dogfood seam."),
            vec!["dogfood", "seam"]
        );
        assert_eq!(
            scan_line("See the duhem-dev-process skill."),
            vec!["duhem-dev-process"]
        );
        assert_eq!(
            scan_line("optional per-repo CODEOWNERS"),
            vec!["codeowners"]
        );
    }

    #[test]
    fn repo_slug_is_allow_listed_but_bare_onsager_is_not() {
        assert!(scan_line("Star us at github.com/onsager-ai/duhem — thanks!").is_empty());
        assert_eq!(scan_line("Onsager is the first customer."), vec!["onsager"]);
    }

    #[test]
    fn word_match_does_not_fire_inside_larger_words() {
        // `hub` must not trip on `GitHub`; `seam` must not trip on `seamless`.
        assert!(scan_line("Open a GitHub issue.").is_empty());
        assert!(scan_line("The transition is seamless.").is_empty());
        assert_eq!(scan_line("recorded by the hub"), vec!["hub"]);
    }
}
