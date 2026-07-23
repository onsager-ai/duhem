//! DX-drift gate — keeps user-facing DX surfaces current with the
//! product, the way `schema-changelog-check` keeps the CHANGELOG current
//! with the schema.
//!
//! Two checks run under one subcommand:
//!
//! 1. **readme-framing** (always, hard): the adoption README a product
//!    repo copies (`templates/product-repo/README.md`) must not carry
//!    internal framing — dogfood/customer names, the trust-model `seam`
//!    vocabulary, or `docs/duhem-spec.md` section refs a consumer's repo
//!    doesn't have. `CODEOWNERS` / `hub` / `duhem ship` are *legitimate*
//!    adoption features and are NOT flagged (this is why it can't reuse
//!    `skill-scrub`'s stricter denylist).
//!
//! 2. **currency** (diff-based, warn-first): if a PR changes a file that
//!    *declares user-visible surface* (the VD schema, the action
//!    catalog/`with:` params, the generated action reference, the CLI
//!    command defs) without touching any hand-maintained DX doc (the
//!    authoring skill, the adoption template, getting-started, the spec,
//!    the root README), it's a reminder that the DX surface may be stale.
//!    Narrow by design: it does NOT fire on internal refactors of those
//!    crates. Silence a false positive with `DUHEM_DX_IMPACT_NONE=1` (CI
//!    sets it when the PR body declares `DX impact: none`).
//!
//! ## Modes
//!
//! - `--mode=warn` (default): print currency reminders, exit 0. The
//!   readme-framing check still hard-fails.
//! - `--mode=fail`: currency reminders also fail the build.
//!
//! ## Comparison base
//!
//! Diffs against `origin/main` (or `origin/$GITHUB_BASE_REF`). If the
//! base can't be resolved (e.g. a local `just check` with no fetched
//! base), the currency check skips gracefully — only readme-framing runs.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

/// The adoption README that ships to product repos.
const ADOPTION_README: &str = "templates/product-repo/README.md";

/// Internal-framing tokens that must never appear in the adoption
/// README. Deliberately narrower than `skill-scrub`'s denylist:
/// `CODEOWNERS`/`hub`/`ship` are real adoption features here, so they're
/// absent. `(needle, whole_word)`.
const README_DENY: &[(&str, bool)] = &[
    ("dogfood", false),
    ("chreode", false),
    ("crawlab", false),
    ("asymmetric", false),
    ("docs/duhem-spec.md", false),
    ("seam", true),
];

/// Files that *declare* user-visible surface. A change here is what arms
/// the currency check. Narrow by design — surface-declaring files, not
/// whole crates. Matched as path prefixes.
const PRODUCT_SURFACE: &[&str] = &[
    "crates/duhem-schema/src/",           // authored VD shape
    "crates/duhem-actions/src/action.rs", // action-kind registry
    "crates/duhem-actions/src/with.rs",   // action `with:` params
    "crates/duhem-cli/src/main.rs",       // top-level CLI commands/flags
    "docs/action-reference.md",           // generated action surface
];

/// Hand-maintained DX docs whose touch means the author considered the
/// DX surface. `docs/action-reference.md` is deliberately NOT here — it
/// is generated, so touching it doesn't mean authoring guidance moved.
const DX_SURFACE: &[&str] = &[
    "templates/product-repo/",
    ".claude/skills/verification-authoring/",
    "docs/getting-started.md",
    "docs/duhem-spec.md",
    "README.md",
];

const OVERRIDE_ENV: &str = "DUHEM_DX_IMPACT_NONE";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Warn,
    Fail,
}

pub fn run(args: Vec<String>) -> Result<()> {
    let mode = parse_mode(args)?;
    let root = workspace_root()?;

    // (1) readme-framing — always, hard.
    readme_framing(&root)?;

    // (2) currency — diff-based, warn-first, skips if base unresolved.
    currency(&root, mode)
}

fn readme_framing(root: &Path) -> Result<()> {
    let path = root.join(ADOPTION_README);
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("dx-drift: {ADOPTION_README} not found; skipping readme-framing");
            return Ok(());
        }
    };
    let mut hits: Vec<(usize, &str)> = Vec::new();
    for (idx, line) in src.lines().enumerate() {
        let low = line.to_lowercase();
        for (needle, whole_word) in README_DENY {
            let found = if *whole_word {
                contains_word(&low, needle)
            } else {
                low.contains(needle)
            };
            if found {
                hits.push((idx + 1, needle));
            }
        }
    }
    if !hits.is_empty() {
        eprintln!("dx-drift readme-framing — internal framing in the adoption README:");
        for (line, token) in &hits {
            eprintln!("  {ADOPTION_README}:{line}: `{token}`");
        }
        eprintln!(
            "\nThe adoption README is user-facing (a product repo copies it). Rewrite the\ninternal framing (dogfood/customer names, `seam`, `docs/duhem-spec.md` refs) into\nuser-facing language. `CODEOWNERS`/`hub`/`ship` are fine — they're real features."
        );
        bail!(
            "{} internal-framing leak(s) in {ADOPTION_README}",
            hits.len()
        );
    }
    Ok(())
}

fn currency(root: &Path, mode: Mode) -> Result<()> {
    let base = comparison_base();
    let changed = match changed_files(root, &base) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("dx-drift: cannot diff against {base}; skipping currency check");
            return Ok(());
        }
    };

    let product: Vec<&String> = changed
        .iter()
        .filter(|p| PRODUCT_SURFACE.iter().any(|s| p.starts_with(s)))
        .collect();
    if product.is_empty() {
        eprintln!("dx-drift: no user-visible surface changed vs {base}; nothing to check");
        return Ok(());
    }

    let dx_touched = changed
        .iter()
        .any(|p| DX_SURFACE.iter().any(|s| p.starts_with(s)));
    if dx_touched {
        eprintln!("dx-drift: surface changed and a DX doc was updated in the same change — OK");
        return Ok(());
    }

    if std::env::var(OVERRIDE_ENV).is_ok_and(|v| !v.is_empty() && v != "0") {
        eprintln!("dx-drift: surface changed with no DX update, but PR declared `DX impact: none`");
        return Ok(());
    }

    eprintln!("dx-drift: user-visible surface changed with no DX doc updated:");
    for p in &product {
        eprintln!("  - surface: {p}");
    }
    eprintln!(
        "\nUpdate a DX surface in this PR (authoring skill, adoption template,\ngetting-started, spec, or README), OR declare `## DX impact` with an explicit\n`none (rationale)` (CI reads it into {OVERRIDE_ENV})."
    );

    match mode {
        Mode::Warn => {
            eprintln!("\n(warn-only mode — not failing the build)");
            Ok(())
        }
        Mode::Fail => bail!(
            "{} surface file(s) changed without a DX update",
            product.len()
        ),
    }
}

fn parse_mode(args: Vec<String>) -> Result<Mode> {
    let mut mode = Mode::Warn;
    let mut iter = args.into_iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--mode=warn" => mode = Mode::Warn,
            "--mode=fail" => mode = Mode::Fail,
            "--mode" => match iter.next().as_deref() {
                Some("warn") => mode = Mode::Warn,
                Some("fail") => mode = Mode::Fail,
                other => bail!("--mode expects warn|fail, got {other:?}"),
            },
            other => bail!("unknown arg: {other}"),
        }
    }
    Ok(mode)
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

    /// Mirror of the readme-framing scan over an in-memory string, so
    /// the denylist logic is testable without touching the filesystem.
    fn framing_hits(src: &str) -> Vec<&'static str> {
        let mut hits = Vec::new();
        for line in src.lines() {
            let low = line.to_lowercase();
            for (needle, whole_word) in README_DENY {
                let found = if *whole_word {
                    contains_word(&low, needle)
                } else {
                    low.contains(needle)
                };
                if found {
                    hits.push(*needle);
                }
            }
        }
        hits
    }

    #[test]
    fn adoption_features_are_not_framing() {
        // CODEOWNERS / hub / ship are legitimate here — must not fire.
        assert!(framing_hits("Hub-recorded verdicts via `duhem ship`.").is_empty());
        assert!(framing_hits("CODEOWNERS routes /.duhem/ edits.").is_empty());
    }

    #[test]
    fn internal_framing_is_caught() {
        assert_eq!(
            framing_hits("This is the reframed dogfood."),
            vec!["dogfood"]
        );
        assert_eq!(framing_hits("not a trust seam (§11.2)"), vec!["seam"]);
        assert_eq!(
            framing_hits("see `docs/duhem-spec.md` §10.1"),
            vec!["docs/duhem-spec.md"]
        );
    }

    #[test]
    fn seam_is_word_bounded() {
        // `seamless` must not trip the `seam` word rule.
        assert!(framing_hits("The adoption is seamless.").is_empty());
    }

    #[test]
    fn surface_and_dx_prefix_sets_are_disjoint_on_generated_ref() {
        // action-reference.md is a trigger (PRODUCT_SURFACE), never a
        // satisfying DX doc — otherwise regenerating it would silence
        // the gate without any authoring update.
        assert!(PRODUCT_SURFACE.contains(&"docs/action-reference.md"));
        assert!(
            !DX_SURFACE
                .iter()
                .any(|s| "docs/action-reference.md".starts_with(s))
        );
    }
}
