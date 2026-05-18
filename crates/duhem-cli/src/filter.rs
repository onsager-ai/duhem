//! `duhem run --filter` pattern parsing and matching.
//!
//! Spec on issue #23. The v1 grammar is deliberately tiny:
//!
//! - `AC-1` — every check under criterion `AC-1`.
//! - `AC-1::AC-1.2` — exactly one `(criterion, check)` pair.
//! - `AC-*` — glob on criterion id; `AC-1::AC-1.*` glob on check id.
//!
//! Multiple `--filter` flags OR together. The implementation lives in
//! the CLI crate because the grammar is a CLI concern; the engine
//! consumes the result via the `duhem_runtime::CheckFilter` trait.

use duhem_runtime::CheckFilter;

/// One parsed `--filter` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterPattern {
    /// Criterion id glob. Must be non-empty.
    criterion: String,
    /// Check id glob. `None` means "all checks under the criterion".
    check: Option<String>,
}

impl FilterPattern {
    fn matches(&self, criterion_id: &str, check_id: &str) -> bool {
        if !glob_match(&self.criterion, criterion_id) {
            return false;
        }
        match &self.check {
            None => true,
            Some(g) => glob_match(g, check_id),
        }
    }
}

/// Parse one `--filter` value. Returns a user-facing error message on
/// the empty-id edge cases the spec explicitly rejects.
pub fn parse_pattern(spec: &str) -> Result<FilterPattern, String> {
    match spec.split_once("::") {
        None => {
            if spec.is_empty() {
                return Err("--filter: empty pattern".to_string());
            }
            Ok(FilterPattern {
                criterion: spec.to_string(),
                check: None,
            })
        }
        Some((c, k)) => {
            if c.is_empty() {
                return Err(format!("--filter `{spec}`: empty criterion id"));
            }
            if k.is_empty() {
                return Err(format!("--filter `{spec}`: empty check id"));
            }
            // v1 grammar permits at most one `::` separator. A
            // remaining `::` in either half would silently match
            // nothing under the glob, so reject it with a clear
            // error rather than letting authors debug an empty run.
            if c.contains("::") || k.contains("::") {
                return Err(format!(
                    "--filter `{spec}`: malformed pattern (at most one `::` separator)"
                ));
            }
            Ok(FilterPattern {
                criterion: c.to_string(),
                check: Some(k.to_string()),
            })
        }
    }
}

/// OR-of-patterns filter. `parse` does *not* itself reject an empty
/// pattern list — an empty `CliCheckFilter` is a well-defined
/// "matches nothing" predicate — but `main.rs` skips constructing
/// one when the user passed no `--filter` flags, so the empty case
/// never reaches the engine in practice.
#[derive(Debug, Clone)]
pub struct CliCheckFilter {
    patterns: Vec<FilterPattern>,
}

impl CliCheckFilter {
    pub fn parse(specs: &[String]) -> Result<Self, String> {
        let mut patterns = Vec::with_capacity(specs.len());
        for s in specs {
            patterns.push(parse_pattern(s)?);
        }
        Ok(Self { patterns })
    }
}

impl CheckFilter for CliCheckFilter {
    fn matches(&self, criterion_id: &str, check_id: &str) -> bool {
        self.patterns
            .iter()
            .any(|p| p.matches(criterion_id, check_id))
    }
}

/// Minimal glob: `*` matches zero or more characters; every other byte
/// matches itself. No `?`, no character classes — the spec calls those
/// out explicitly as out of scope. Recursion depth is bounded by the
/// number of `*` in the pattern, which authors keep small.
fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_bytes(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_bytes(p: &[u8], t: &[u8]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    if p[0] == b'*' {
        let mut rest = &p[1..];
        while rest.first() == Some(&b'*') {
            rest = &rest[1..];
        }
        for i in 0..=t.len() {
            if glob_match_bytes(rest, &t[i..]) {
                return true;
            }
        }
        false
    } else if !t.is_empty() && p[0] == t[0] {
        glob_match_bytes(&p[1..], &t[1..])
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_criterion_only_pattern() {
        let p = parse_pattern("AC-1").unwrap();
        assert_eq!(p.criterion, "AC-1");
        assert_eq!(p.check, None);
    }

    #[test]
    fn parses_pair_pattern() {
        let p = parse_pattern("AC-1::AC-1.2").unwrap();
        assert_eq!(p.criterion, "AC-1");
        assert_eq!(p.check.as_deref(), Some("AC-1.2"));
    }

    #[test]
    fn parses_globbed_pair() {
        let p = parse_pattern("AC-*::AC-*.1").unwrap();
        assert_eq!(p.criterion, "AC-*");
        assert_eq!(p.check.as_deref(), Some("AC-*.1"));
    }

    #[test]
    fn rejects_empty_criterion() {
        let err = parse_pattern("::AC-1.1").unwrap_err();
        assert!(err.contains("empty criterion"), "got: {err}");
    }

    #[test]
    fn rejects_empty_check() {
        let err = parse_pattern("AC-1::").unwrap_err();
        assert!(err.contains("empty check"), "got: {err}");
    }

    #[test]
    fn rejects_multiple_separators() {
        // v1 grammar allows at most one `::`. A typo'd third
        // component (e.g. `AC-1::AC-1.1::typo`) would otherwise
        // silently match nothing — surface it as a parse error.
        let err = parse_pattern("AC-1::AC-1.1::typo").unwrap_err();
        assert!(err.contains("malformed pattern"), "got: {err}");
    }

    #[test]
    fn criterion_only_matches_all_checks_under_it() {
        let f = CliCheckFilter::parse(&["AC-1".to_string()]).unwrap();
        assert!(f.matches("AC-1", "AC-1.1"));
        assert!(f.matches("AC-1", "AC-1.2"));
        assert!(!f.matches("AC-2", "AC-2.1"));
    }

    #[test]
    fn pair_matches_exactly() {
        let f = CliCheckFilter::parse(&["AC-1::AC-1.2".to_string()]).unwrap();
        assert!(f.matches("AC-1", "AC-1.2"));
        assert!(!f.matches("AC-1", "AC-1.1"));
        assert!(!f.matches("AC-2", "AC-1.2"));
    }

    #[test]
    fn glob_pair_matches_globally() {
        let f = CliCheckFilter::parse(&["AC-*::AC-*.1".to_string()]).unwrap();
        assert!(f.matches("AC-1", "AC-1.1"));
        assert!(f.matches("AC-2", "AC-2.1"));
        assert!(!f.matches("AC-1", "AC-1.2"));
    }

    #[test]
    fn multiple_patterns_or() {
        let f = CliCheckFilter::parse(&["AC-1".to_string(), "AC-2::AC-2.3".to_string()]).unwrap();
        assert!(f.matches("AC-1", "AC-1.1"));
        assert!(f.matches("AC-2", "AC-2.3"));
        assert!(!f.matches("AC-2", "AC-2.1"));
        assert!(!f.matches("AC-3", "AC-3.1"));
    }

    #[test]
    fn glob_matches_empty_run() {
        // `*` should match the empty suffix — corner case in the
        // hand-rolled matcher.
        assert!(glob_match("AC-*", "AC-"));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn glob_anchors_both_ends() {
        // Without anchoring, "AC-1" would substring-match "AC-10";
        // verify the matcher rejects that.
        assert!(!glob_match("AC-1", "AC-10"));
        assert!(!glob_match("AC-1", "0AC-1"));
    }
}
