//! Schema-drift check — fails CI if `docs/duhem-spec.md` §10 contains a
//! YAML example that doesn't parse / validate against the current
//! `duhem_schema::SCHEMA_VERSION`.
//!
//! Per the spec issue that introduced this xtask: §10 is the
//! authoritative reader-facing description of the Verification
//! Definition format. If the spec example no longer parses, either
//! the spec or the code has drifted; we want CI to surface the
//! disagreement, not let either side rot.
//!
//! ## What's checked
//!
//! - All fenced `yaml` / `yml` code blocks under §10 (between
//!   `## 10. ` and the next `## ` heading).
//! - A block is treated as a Verification Definition iff it has a
//!   top-level `verification:` or `criteria:` key (the same
//!   self-identification rule the spec describes in §10.2). Other
//!   blocks (the §10.4 root manifest, snippet fragments) are skipped.
//! - VD blocks are parsed via `VerificationDefinition::from_yaml_str`
//!   and then structurally validated via `duhem_schema::validate`.
//!
//! ## What's not checked
//!
//! - Per-action `with:` schemas. The schema crate keeps `with:`
//!   opaque (`serde_yml::Value`); per-action validation is the
//!   action's own concern at runtime. A Phase-1 deeper check could
//!   add per-action structural validation here.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

const SPEC_PATH: &str = "docs/duhem-spec.md";

pub fn run(_args: Vec<String>) -> Result<()> {
    let root = workspace_root()?;
    let spec_path = root.join(SPEC_PATH);
    let src = std::fs::read_to_string(&spec_path)
        .with_context(|| format!("read {}", spec_path.display()))?;

    let blocks = extract_section_yaml(&src, "10");
    if blocks.is_empty() {
        bail!("no fenced yaml blocks found under §10 — did the spec restructure?");
    }

    let mut failures: Vec<String> = Vec::new();
    let mut vd_count = 0usize;
    let mut skipped = 0usize;

    for block in &blocks {
        if !looks_like_vd(&block.body) {
            skipped += 1;
            continue;
        }
        vd_count += 1;
        match validate_vd(&block.body) {
            Ok(()) => {}
            Err(e) => failures.push(format!(
                "{}:{} (§10 block #{}): {e}",
                SPEC_PATH,
                block.line,
                block.index + 1
            )),
        }
    }

    eprintln!(
        "schema-drift: scanned {} §10 yaml block(s); {vd_count} VD, {skipped} non-VD; schema v{}",
        blocks.len(),
        duhem_schema::SCHEMA_VERSION,
    );

    if !failures.is_empty() {
        for f in &failures {
            eprintln!("  - {f}");
        }
        bail!(
            "{} §10 example(s) failed to parse/validate against schema v{}",
            failures.len(),
            duhem_schema::SCHEMA_VERSION
        );
    }
    Ok(())
}

struct Block {
    /// 1-based line where the opening ```` ``` ```` fence appears.
    line: usize,
    /// 0-based position of this block within the section.
    index: usize,
    body: String,
}

/// Return all fenced `yaml` / `yml` blocks that appear between a
/// `## <number>. ` heading and the next `## ` heading. Headings are
/// matched on the leading number only — section title text after the
/// number can change without breaking the check.
fn extract_section_yaml(src: &str, section: &str) -> Vec<Block> {
    let lines: Vec<&str> = src.lines().collect();
    let mut start: Option<usize> = None;
    let mut end: Option<usize> = None;
    let prefix = format!("## {section}. ");
    for (i, line) in lines.iter().enumerate() {
        if start.is_none() && line.starts_with(&prefix) {
            start = Some(i);
            continue;
        }
        if start.is_some() && line.starts_with("## ") && !line.starts_with(&prefix) {
            end = Some(i);
            break;
        }
    }
    let Some(s) = start else { return Vec::new() };
    let e = end.unwrap_or(lines.len());

    let mut out: Vec<Block> = Vec::new();
    let mut in_block = false;
    let mut buf = String::new();
    let mut open_line = 0usize;
    let mut idx = 0usize;
    for (i, line) in lines[s..e].iter().enumerate() {
        let trimmed = line.trim_start();
        if !in_block {
            if trimmed.starts_with("```yaml") || trimmed.starts_with("```yml") {
                in_block = true;
                buf.clear();
                open_line = s + i + 1; // 1-based line of the opening fence
            }
        } else if trimmed.starts_with("```") {
            out.push(Block {
                line: open_line,
                index: idx,
                body: std::mem::take(&mut buf),
            });
            idx += 1;
            in_block = false;
        } else {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    out
}

/// Top-level self-identification per spec §10.2. A YAML document is a
/// Verification Definition iff it carries a top-level `verification:`
/// or `criteria:` key. Avoids running the VD validator on the §10.4
/// root-manifest example (which has neither).
///
/// Uses a non-parsing line scan deliberately: if a §10 block looks
/// like a VD but the YAML is malformed, we want `validate_vd` to
/// surface the parse error as a CI failure, not silently skip.
/// Parsing here would swallow the error and let broken examples
/// through.
fn looks_like_vd(body: &str) -> bool {
    for raw in body.lines() {
        // Strip trailing comments so a key embedded in a comment
        // doesn't trip the scan; YAML escapes inside `#` are out of
        // scope for spec examples.
        let line = match raw.split_once('#') {
            Some((before, _)) => before,
            None => raw,
        };
        // Top-level keys sit at column 0; an indented `verification:`
        // is some nested field, not the document's identity.
        if line.starts_with("verification:") || line.starts_with("criteria:") {
            return true;
        }
    }
    false
}

fn validate_vd(body: &str) -> Result<()> {
    let v = duhem_schema::VerificationDefinition::from_yaml_str(body)
        .map_err(|e| anyhow!("parse: {e}"))?;
    duhem_schema::validate(&v).map_err(|errs| {
        let mut s = format!("{} validation error(s):", errs.len());
        for e in errs {
            s.push_str("\n      - ");
            s.push_str(&e.to_string());
        }
        anyhow!(s)
    })
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
    fn extracts_yaml_blocks_under_section() {
        let src = "\
## 9. Other
```yaml
verification: skip-me
criteria: []
```
## 10. The Verification Definition
prose
```yaml
verification: ok
criteria: []
```
```yml
verification: also-ok
criteria: []
```
not yaml ```js
const x = 1;
```
## 11. Architecture
```yaml
verification: out-of-section
criteria: []
```
";
        let blocks = extract_section_yaml(src, "10");
        assert_eq!(blocks.len(), 2, "blocks: {:?}", blocks.len());
        assert!(blocks[0].body.contains("verification: ok"));
        assert!(blocks[1].body.contains("verification: also-ok"));
        assert_eq!(blocks[0].index, 0);
        assert_eq!(blocks[1].index, 1);
        assert!(blocks[0].line > 0);
    }

    #[test]
    fn looks_like_vd_distinguishes_manifest_from_vd() {
        let vd = "verification: x\ncriteria: []\n";
        let manifest = "version: \"1\"\nverifications:\n  - a.yml\n";
        let scrap = "- a\n- b\n";
        assert!(looks_like_vd(vd));
        assert!(!looks_like_vd(manifest));
        assert!(!looks_like_vd(scrap));
    }

    #[test]
    fn looks_like_vd_accepts_criteria_only() {
        let only_criteria = "criteria:\n  - id: AC-1\n    description: x\n    checks: []\n";
        assert!(looks_like_vd(only_criteria));
    }

    #[test]
    fn looks_like_vd_ignores_indented_keys() {
        // A nested `verification:` inside another structure must not
        // be misread as the document's identity.
        let nested = "version: \"1\"\nverifications:\n  - file.yml\nmeta:\n  verification: false\n";
        assert!(!looks_like_vd(nested));
    }

    #[test]
    fn looks_like_vd_routes_malformed_yaml_to_validator() {
        // The scan must not swallow parse errors: a block that
        // *looks* like a VD (top-level `verification:`) but has
        // broken YAML body still classifies as VD, so the validator
        // gets to surface the parse error as a CI failure.
        let broken = "verification: x\ncriteria: [unterminated\n";
        assert!(looks_like_vd(broken));
        assert!(validate_vd(broken).is_err());
    }

    #[test]
    fn validates_a_minimal_vd() {
        let body = "\
verification: minimal
criteria:
  - id: AC-1
    description: trivial
    checks:
      - id: AC-1.1
        assertions:
          - $inputs.x == 1
inputs:
  x:
    type: integer
    default: 1
";
        validate_vd(body).expect("should validate");
    }
}
