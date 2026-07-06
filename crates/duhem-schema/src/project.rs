//! `project:` — the declared target coordinate (#191).
//!
//! A Verification Definition (or a root manifest, suite-wide)
//! optionally declares **what it verifies**: a repo, a deployed
//! service, an image, or a locally-named project. The declaration is
//! the top rung of the #188 identity ladder — the runtime's
//! resolution order is declared `project:` → CI context → normalized
//! `origin` remote → path fallback — and populates the store's
//! `project_id` hint plus the `target_repo` provenance column (#190).
//!
//! Exactly one coordinate field must be set; the field chosen *is*
//! the kind (`repo:` = git, `url:` = url, `image:` = image, `id:` =
//! custom). Identity never derives from a root commit (rejected on
//! #188).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The declared target coordinate. Wire shape:
///
/// ```yaml
/// project:
///   repo: github.com/crawlab-team/crawlab-pro   # git kind, OR
///   # url: https://app.example.com              # url kind, OR
///   # image: registry.example.com/app:tag       # image kind, OR
///   # id: crawlab-pro                           # custom kind
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectDecl {
    /// Custom kind: a human-stable local name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Git kind: a forge coordinate (`host/owner/repo`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Url kind: a deployed service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Image kind: a container image reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// The kind a declaration resolved to — named by the field that was
/// set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectKind {
    Git,
    Url,
    Image,
    Custom,
}

impl ProjectDecl {
    /// The declared coordinate and its kind, when the block is
    /// well-formed (exactly one non-empty field).
    pub fn coordinate(&self) -> Option<(ProjectKind, &str)> {
        let mut found: Option<(ProjectKind, &str)> = None;
        for (kind, value) in [
            (ProjectKind::Git, &self.repo),
            (ProjectKind::Url, &self.url),
            (ProjectKind::Image, &self.image),
            (ProjectKind::Custom, &self.id),
        ] {
            if let Some(v) = value.as_deref() {
                if found.is_some() {
                    return None; // more than one field set
                }
                found = Some((kind, v));
            }
        }
        found.filter(|(_, v)| !v.trim().is_empty())
    }

    /// Structural rule: exactly one of `repo:` / `url:` / `image:` /
    /// `id:`, non-empty. Returned as a message the caller wraps into
    /// its own error type (leaf `validate` or manifest load).
    pub fn check(&self) -> Result<(), String> {
        let set = [&self.repo, &self.url, &self.image, &self.id]
            .iter()
            .filter(|f| f.is_some())
            .count();
        match set {
            0 => Err(
                "project: declares no coordinate; set exactly one of `repo:` (git), \
                 `url:`, `image:`, or `id:` (custom)"
                    .to_string(),
            ),
            1 => match self.coordinate() {
                Some(_) => Ok(()),
                None => Err("project: coordinate must be non-empty".to_string()),
            },
            n => Err(format!(
                "project: declares {n} coordinates; set exactly one of `repo:`, `url:`, \
                 `image:`, or `id:`"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(yaml: &str) -> ProjectDecl {
        serde_yml::from_str(yaml).expect("parse")
    }

    #[test]
    fn each_kind_round_trips_and_reports_its_coordinate() {
        for (yaml, kind, coord) in [
            (
                "repo: github.com/crawlab-team/crawlab-pro",
                ProjectKind::Git,
                "github.com/crawlab-team/crawlab-pro",
            ),
            (
                "url: https://app.example.com",
                ProjectKind::Url,
                "https://app.example.com",
            ),
            (
                "image: registry/app:tag",
                ProjectKind::Image,
                "registry/app:tag",
            ),
            ("id: crawlab-pro", ProjectKind::Custom, "crawlab-pro"),
        ] {
            let d = decl(yaml);
            assert!(d.check().is_ok(), "{yaml}");
            assert_eq!(d.coordinate(), Some((kind, coord)), "{yaml}");
        }
    }

    #[test]
    fn zero_or_multiple_coordinates_are_rejected() {
        assert!(decl("{}").check().is_err());
        assert!(
            decl("repo: a/b\nid: also")
                .check()
                .unwrap_err()
                .contains("2 coordinates")
        );
        assert!(decl("id: \"  \"").check().is_err(), "blank coordinate");
    }

    #[test]
    fn unknown_fields_parse_fail() {
        assert!(serde_yml::from_str::<ProjectDecl>("kind: git\nrepo: a/b").is_err());
    }
}
