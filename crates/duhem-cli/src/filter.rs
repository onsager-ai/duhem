//! `duhem run --filter` pattern parsing and matching.
//!
//! Spec on issue #23 (v1 grammar) and issue #49 (optional verification
//! axis when running a root manifest). The v1.5 grammar:
//!
//! - `AC-1` — every check under criterion `AC-1` in every leaf.
//! - `AC-1::AC-1.2` — exactly one `(criterion, check)` pair in every
//!   leaf.
//! - `<verification>::AC-1::AC-1.2` — exactly one
//!   `(verification, criterion, check)` triple. `<verification>` is
//!   the leaf's directory name when running a root manifest; for a
//!   single-leaf invocation it matches against the leaf's parent
//!   directory or the `verification:` field, whichever the loader
//!   recorded.
//! - Glob `*` is allowed in every axis (`*::AC-*::AC-*.1` etc.).
//!
//! Multiple `--filter` flags OR together. The implementation lives in
//! the CLI crate because the grammar is a CLI concern; the engine
//! consumes the result via the `duhem_runtime::CheckFilter` trait,
//! which sees only `(criterion, check)` — the verification axis is
//! resolved CLI-side by `for_verification` before the per-leaf engine
//! sees the filter.

use duhem_runtime::CheckFilter;

/// One parsed `--filter` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterPattern {
    /// Optional verification name glob. `None` means "matches every
    /// verification" (the pre-#49 two-part form).
    verification: Option<String>,
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

    /// Does this pattern's verification axis (if any) match `name`?
    /// `None` means "applies to every verification."
    fn matches_verification(&self, name: &str) -> bool {
        match &self.verification {
            None => true,
            Some(g) => glob_match(g, name),
        }
    }
}

/// Parse one `--filter` value. Returns a user-facing error message on
/// the empty-id edge cases the spec explicitly rejects.
///
/// The grammar is positional on `::` separators:
///
/// - 1 part — criterion glob.
/// - 2 parts — `criterion::check`.
/// - 3 parts — `verification::criterion::check` (issue #49).
///
/// Four-or-more parts is a typo — surface it rather than silently
/// matching nothing.
pub fn parse_pattern(spec: &str) -> Result<FilterPattern, String> {
    if spec.is_empty() {
        return Err("--filter: empty pattern".to_string());
    }
    let parts: Vec<&str> = spec.split("::").collect();
    match parts.as_slice() {
        [c] => Ok(FilterPattern {
            verification: None,
            criterion: (*c).to_string(),
            check: None,
        }),
        [c, k] => {
            if c.is_empty() {
                return Err(format!("--filter `{spec}`: empty criterion id"));
            }
            if k.is_empty() {
                return Err(format!("--filter `{spec}`: empty check id"));
            }
            Ok(FilterPattern {
                verification: None,
                criterion: (*c).to_string(),
                check: Some((*k).to_string()),
            })
        }
        [v, c, k] => {
            if v.is_empty() {
                return Err(format!("--filter `{spec}`: empty verification id"));
            }
            if c.is_empty() {
                return Err(format!("--filter `{spec}`: empty criterion id"));
            }
            if k.is_empty() {
                return Err(format!("--filter `{spec}`: empty check id"));
            }
            Ok(FilterPattern {
                verification: Some((*v).to_string()),
                criterion: (*c).to_string(),
                check: Some((*k).to_string()),
            })
        }
        _ => Err(format!(
            "--filter `{spec}`: malformed pattern (expected `[verification::]criterion[::check]`)"
        )),
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

    /// Narrow this filter to one verification by name. Patterns that
    /// either have no verification axis (the two-part form means "all
    /// verifications") or whose verification glob matches `name`
    /// survive — the rest are dropped. Spec on issue #49.
    ///
    /// Returns `None` when no patterns survive; the caller treats
    /// that as "skip this leaf entirely" rather than instantiating an
    /// empty filter that would match nothing on the engine side. This
    /// preserves the spec's "filter parse failures surface before
    /// browser launch" property: if every pattern was scoped to a
    /// different leaf, we don't pay the per-leaf launch cost on the
    /// non-matching ones.
    pub fn for_verification(&self, name: &str) -> Option<Self> {
        let patterns: Vec<FilterPattern> = self
            .patterns
            .iter()
            .filter(|p| p.matches_verification(name))
            .cloned()
            .collect();
        if patterns.is_empty() {
            None
        } else {
            Some(Self { patterns })
        }
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
        assert_eq!(p.verification, None);
        assert_eq!(p.criterion, "AC-1");
        assert_eq!(p.check.as_deref(), Some("AC-1.2"));
    }

    #[test]
    fn parses_triple_with_verification_axis() {
        // Spec on #49: `verification::criterion::check` selects in
        // the named leaf.
        let p = parse_pattern("foo::AC-1::AC-1.2").unwrap();
        assert_eq!(p.verification.as_deref(), Some("foo"));
        assert_eq!(p.criterion, "AC-1");
        assert_eq!(p.check.as_deref(), Some("AC-1.2"));
    }

    #[test]
    fn two_part_pattern_means_every_verification() {
        // Spec on #49 § Alignment "`--filter` grammar extension": old
        // two-part form keeps its "all verifications" meaning.
        let p = parse_pattern("AC-1::AC-1.1").unwrap();
        assert!(p.matches_verification("anything"));
        assert!(p.matches_verification("else"));
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
    fn rejects_more_than_three_components() {
        // Three components is the new ceiling (#49). A typo'd fourth
        // component would otherwise silently match nothing — surface
        // it as a parse error.
        let err = parse_pattern("foo::AC-1::AC-1.1::typo").unwrap_err();
        assert!(err.contains("malformed pattern"), "got: {err}");
    }

    #[test]
    fn rejects_empty_verification_axis() {
        let err = parse_pattern("::AC-1::AC-1.1").unwrap_err();
        assert!(err.contains("empty verification"), "got: {err}");
    }

    #[test]
    fn for_verification_drops_non_matching_patterns() {
        // Spec on #49 Test: triple-form filter must only match its
        // named leaf; two-part form must match everywhere.
        let f =
            CliCheckFilter::parse(&["foo::AC-1::AC-1.1".to_string(), "AC-2::AC-2.3".to_string()])
                .unwrap();
        let in_foo = f.for_verification("foo").expect("matches foo");
        assert!(in_foo.matches("AC-1", "AC-1.1"));
        assert!(in_foo.matches("AC-2", "AC-2.3"));
        // In a leaf named "bar", the `foo::...` pattern drops out
        // but the two-part one survives.
        let in_bar = f.for_verification("bar").expect("two-part survives");
        assert!(!in_bar.matches("AC-1", "AC-1.1"));
        assert!(in_bar.matches("AC-2", "AC-2.3"));
    }

    #[test]
    fn for_verification_returns_none_when_no_pattern_applies() {
        let f = CliCheckFilter::parse(&["foo::AC-1::AC-1.1".to_string()]).unwrap();
        assert!(
            f.for_verification("bar").is_none(),
            "leaf with no surviving patterns should be skippable"
        );
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
