//! Target-identity resolution (#191): populate the store's scoping +
//! provenance (#190) from the identity ladder decided on #188.
//!
//! **Target** (what the run verifies): declared `project:` → CI
//! context (`DUHEM_TARGET_REPO`/`DUHEM_TARGET_SHA`, else
//! `GITHUB_REPOSITORY` + `GITHUB_SHA`) → normalized `origin` remote
//! of the working copy → canonical-path fallback. **Verifier** (where
//! the VD lives): `DUHEM_VERIFIER_REPO`/`DUHEM_VERIFIER_SHA` → git
//! context of the VD's directory. Identity never derives from a root
//! commit (rejected on #188 — shallow clones omit it, forks collide,
//! rewrites re-key).
//!
//! `project_id` is the target coordinate **hint stored as-is** (#190
//! decision); the hub reconciles hints to forge repo-IDs (#188).
//! Resolution is pure over an injected environment + git lookup so
//! the ladder is unit-testable; the production wrapper shells out to
//! `git` and degrades to `None` when a rung is unavailable.

use std::path::Path;

use duhem_evidence::RunScope;
use duhem_schema::ProjectDecl;

/// Environment + git facts the ladder consumes. Injected so tests
/// exercise precedence without process-global state.
pub(crate) trait IdentitySource {
    fn env(&self, key: &str) -> Option<String>;
    /// Normalized `origin` remote (`host/owner/repo`) of the git
    /// working copy containing `dir`, if any.
    fn remote(&self, dir: &Path) -> Option<String>;
    /// `HEAD` commit sha of the working copy containing `dir`.
    fn head_sha(&self, dir: &Path) -> Option<String>;
}

/// Production source: process env + `git` subprocess lookups.
pub(crate) struct ProcessIdentitySource;

impl IdentitySource for ProcessIdentitySource {
    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.trim().is_empty())
    }
    fn remote(&self, dir: &Path) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["remote", "get-url", "origin"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        normalize_remote(&raw)
    }
    fn head_sha(&self, dir: &Path) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!sha.is_empty()).then_some(sha)
    }
}

/// Resolve the run's scope: the CLI calls this once per leaf and
/// threads the result through `Engine::with_scope`.
///
/// `declared` is the effective `project:` (leaf wins over manifest —
/// the caller applies that precedence). `workdir` anchors the
/// target-side git fallback; `vd_dir` anchors the verifier side (the
/// repo the VD lives in), falling back to `workdir`.
pub fn resolve_scope(
    declared: Option<&ProjectDecl>,
    workdir: &Path,
    vd_dir: Option<&Path>,
) -> RunScope {
    resolve_scope_with(&ProcessIdentitySource, declared, workdir, vd_dir)
}

pub(crate) fn resolve_scope_with(
    source: &dyn IdentitySource,
    declared: Option<&ProjectDecl>,
    workdir: &Path,
    vd_dir: Option<&Path>,
) -> RunScope {
    // --- target: the #188 ladder, top rung first -------------------
    let ci_repo = source
        .env("DUHEM_TARGET_REPO")
        .or_else(|| {
            source
                .env("GITHUB_REPOSITORY")
                .map(|r| format!("github.com/{r}"))
        })
        .filter(|v| !v.is_empty());
    let ci_sha = source
        .env("DUHEM_TARGET_SHA")
        .or_else(|| source.env("GITHUB_SHA"));

    let declared_coord = declared
        .and_then(|d| d.coordinate())
        .map(|(_, coord)| coord.to_string());

    let (project_id, target_repo, target_sha) = if let Some(coord) = declared_coord {
        // Declared wins for identity; the CI sha still dates the run
        // when present (the declaration names the project, the CI
        // context names the revision under test).
        (Some(coord.clone()), Some(coord), ci_sha)
    } else if let Some(repo) = ci_repo {
        (Some(repo.clone()), Some(repo), ci_sha)
    } else if let Some(remote) = source.remote(workdir) {
        (Some(remote.clone()), Some(remote), source.head_sha(workdir))
    } else {
        // Path fallback: the canonical working-copy path is the hint.
        // No repo coordinate, no sha — an honest "local, unpublished".
        let path_hint = std::fs::canonicalize(workdir)
            .unwrap_or_else(|_| workdir.to_path_buf())
            .to_string_lossy()
            .into_owned();
        (Some(path_hint), None, None)
    };

    // --- verifier: where the VD lives -------------------------------
    let verifier_dir = vd_dir.unwrap_or(workdir);
    let verifier_repo = source
        .env("DUHEM_VERIFIER_REPO")
        .or_else(|| source.remote(verifier_dir));
    let verifier_sha = source
        .env("DUHEM_VERIFIER_SHA")
        .or_else(|| source.head_sha(verifier_dir));

    RunScope {
        project_id,
        verifier_repo,
        verifier_sha,
        target_repo,
        target_sha,
    }
}

/// Normalize a git remote URL to `host/owner/repo`:
/// `git@github.com:owner/repo.git` and
/// `https://github.com/owner/repo.git` both → `github.com/owner/repo`.
pub(crate) fn normalize_remote(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let stripped = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .or_else(|| raw.strip_prefix("ssh://git@"))
        .or_else(|| raw.strip_prefix("git://"));
    let hostpath = match stripped {
        Some(rest) => rest.to_string(),
        None => match raw.strip_prefix("git@") {
            // scp-like: git@host:owner/repo(.git)
            Some(rest) => rest.replacen(':', "/", 1),
            None => return None, // local paths / unknown schemes: no coordinate
        },
    };
    // Drop credentials and a trailing `.git`.
    let hostpath = hostpath
        .rsplit_once('@')
        .map(|(_, h)| h.to_string())
        .unwrap_or(hostpath);
    let hostpath = hostpath.strip_suffix(".git").unwrap_or(&hostpath);
    let hostpath = hostpath.trim_end_matches('/');
    // A plausible coordinate has host + at least one path segment.
    (hostpath.matches('/').count() >= 2 || {
        // host/owner/repo is 2 slashes; host/repo (rare) is 1 — accept.
        hostpath.matches('/').count() >= 1
    })
    .then(|| hostpath.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[derive(Default)]
    struct FakeSource {
        env: HashMap<String, String>,
        remotes: HashMap<PathBuf, String>,
        shas: HashMap<PathBuf, String>,
    }

    impl IdentitySource for FakeSource {
        fn env(&self, key: &str) -> Option<String> {
            self.env.get(key).cloned()
        }
        fn remote(&self, dir: &Path) -> Option<String> {
            self.remotes.get(dir).cloned()
        }
        fn head_sha(&self, dir: &Path) -> Option<String> {
            self.shas.get(dir).cloned()
        }
    }

    fn declared(repo: &str) -> ProjectDecl {
        serde_yml::from_str(&format!("repo: {repo}")).unwrap()
    }

    #[test]
    fn declared_beats_ci_beats_remote_beats_path() {
        let wd = PathBuf::from("/work");
        let mut src = FakeSource::default();
        src.env
            .insert("GITHUB_REPOSITORY".into(), "acme/from-ci".into());
        src.env.insert("GITHUB_SHA".into(), "cisha".into());
        src.remotes
            .insert(wd.clone(), "github.com/acme/from-remote".into());
        src.shas.insert(wd.clone(), "headsha".into());

        // Declared wins for identity; the CI sha still dates the run.
        let d = declared("github.com/crawlab-team/crawlab-pro");
        let scope = resolve_scope_with(&src, Some(&d), &wd, None);
        assert_eq!(
            scope.project_id.as_deref(),
            Some("github.com/crawlab-team/crawlab-pro")
        );
        assert_eq!(
            scope.target_repo.as_deref(),
            Some("github.com/crawlab-team/crawlab-pro")
        );
        assert_eq!(scope.target_sha.as_deref(), Some("cisha"));

        // No declaration → CI context.
        let scope = resolve_scope_with(&src, None, &wd, None);
        assert_eq!(
            scope.target_repo.as_deref(),
            Some("github.com/acme/from-ci")
        );
        assert_eq!(scope.target_sha.as_deref(), Some("cisha"));

        // No CI → remote + HEAD.
        src.env.clear();
        let scope = resolve_scope_with(&src, None, &wd, None);
        assert_eq!(
            scope.target_repo.as_deref(),
            Some("github.com/acme/from-remote")
        );
        assert_eq!(scope.target_sha.as_deref(), Some("headsha"));
        // Self-verifying: the verifier resolves to the same repo.
        assert_eq!(
            scope.verifier_repo.as_deref(),
            Some("github.com/acme/from-remote")
        );

        // No remote → path hint, no repo coordinate, no sha.
        src.remotes.clear();
        src.shas.clear();
        let scope = resolve_scope_with(&src, None, &wd, None);
        assert!(scope.project_id.as_deref().unwrap().ends_with("work"));
        assert_eq!(scope.target_repo, None);
        assert_eq!(scope.target_sha, None);
    }

    #[test]
    fn duhem_env_overrides_beat_github_context() {
        let wd = PathBuf::from("/work");
        let mut src = FakeSource::default();
        src.env
            .insert("GITHUB_REPOSITORY".into(), "acme/target".into());
        src.env.insert("GITHUB_SHA".into(), "ghsha".into());
        src.env
            .insert("DUHEM_TARGET_REPO".into(), "gitlab.com/acme/real".into());
        src.env.insert("DUHEM_TARGET_SHA".into(), "realsha".into());
        src.env.insert(
            "DUHEM_VERIFIER_REPO".into(),
            "github.com/onsager-ai/duhem".into(),
        );
        src.env.insert("DUHEM_VERIFIER_SHA".into(), "v0.1.0".into());

        let scope = resolve_scope_with(&src, None, &wd, None);
        assert_eq!(scope.target_repo.as_deref(), Some("gitlab.com/acme/real"));
        assert_eq!(scope.target_sha.as_deref(), Some("realsha"));
        assert_eq!(
            scope.verifier_repo.as_deref(),
            Some("github.com/onsager-ai/duhem")
        );
        assert_eq!(scope.verifier_sha.as_deref(), Some("v0.1.0"));
    }

    #[test]
    fn verifier_resolves_from_the_vd_directory() {
        let wd = PathBuf::from("/work/target");
        let vd = PathBuf::from("/work/duhem-checkout/verifications/x");
        let mut src = FakeSource::default();
        src.remotes.insert(wd.clone(), "github.com/acme/app".into());
        src.shas.insert(wd.clone(), "appsha".into());
        src.remotes
            .insert(vd.clone(), "github.com/onsager-ai/duhem".into());
        src.shas.insert(vd.clone(), "duhemsha".into());

        let scope = resolve_scope_with(&src, None, &wd, Some(&vd));
        assert_eq!(scope.target_repo.as_deref(), Some("github.com/acme/app"));
        assert_eq!(
            scope.verifier_repo.as_deref(),
            Some("github.com/onsager-ai/duhem")
        );
        assert_eq!(scope.verifier_sha.as_deref(), Some("duhemsha"));
    }

    #[test]
    fn remote_urls_normalize_to_host_owner_repo() {
        for (raw, want) in [
            (
                "git@github.com:owner/repo.git",
                Some("github.com/owner/repo"),
            ),
            (
                "https://github.com/owner/repo.git",
                Some("github.com/owner/repo"),
            ),
            (
                "https://github.com/owner/repo",
                Some("github.com/owner/repo"),
            ),
            (
                "ssh://git@gitlab.com/group/sub/repo.git",
                Some("gitlab.com/group/sub/repo"),
            ),
            (
                "https://user:tok@github.com/owner/repo.git",
                Some("github.com/owner/repo"),
            ),
            ("/home/x/local-repo", None),
            ("", None),
        ] {
            assert_eq!(normalize_remote(raw).as_deref(), want, "raw: {raw}");
        }
    }
}
