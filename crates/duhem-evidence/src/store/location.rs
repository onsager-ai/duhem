//! Store location resolution (#189, decided on #188).
//!
//! The store lives outside the working copy, under the OS-native
//! *state* directory, namespaced per working copy by a readable
//! path-slug plus a short hash:
//!
//! ```text
//! $DUHEM_HOME/projects/<path-slug>-<hash8>/duhem.db          # if DUHEM_HOME set
//! $XDG_STATE_HOME/duhem/projects/<slug>-<hash8>/duhem.db     # Linux default
//! <os state or local-data dir>/duhem/projects/<...>/duhem.db # macOS/Windows
//! ```
//!
//! The key is the canonical working-copy *path* — commit-agnostic (a
//! new commit never re-keys the store) and per-working-copy (two
//! clones or worktrees at different paths get separate stores). The
//! durable cross-machine identity is a #190/#191 concern; this module
//! only answers "where is *this* working copy's DB?".

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::StoreError;

/// Longest readable slug we keep. Long paths keep their *tail* (the
/// most distinctive part); uniqueness is carried by the hash suffix,
/// so truncation can never collide two projects.
const SLUG_MAX: usize = 96;

/// The duhem state root: `$DUHEM_HOME` if set, else the OS state
/// directory (`$XDG_STATE_HOME/duhem` or `~/.local/state/duhem` on
/// Linux; the local-data dir on macOS/Windows, which have no separate
/// state notion).
pub fn state_root() -> Result<PathBuf, StoreError> {
    if let Some(home) = std::env::var_os("DUHEM_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    let base = directories::BaseDirs::new().ok_or(StoreError::NoStateDir)?;
    let root = base
        .state_dir()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| base.data_local_dir().to_path_buf());
    Ok(root.join("duhem"))
}

/// The per-working-copy namespace: readable path-slug + 8-hex-char
/// SHA-256 suffix of the canonical path. `/home/x/proj` →
/// `home-x-proj-3fa2b1c8`.
pub fn project_slug(workdir: &Path) -> String {
    // Canonicalize so `.`, symlinks, and relative invocations of the
    // same working copy key identically. Fall back to the lexical
    // absolute path if the directory can't be resolved.
    let canonical = std::fs::canonicalize(workdir).unwrap_or_else(|_| {
        if workdir.is_absolute() {
            workdir.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(workdir))
                .unwrap_or_else(|_| workdir.to_path_buf())
        }
    });
    let raw = canonical.to_string_lossy();

    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let hash8 = &hex::encode(hasher.finalize())[..8];

    let mut slug: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    slug = slug.trim_matches('-').to_string();
    if slug.len() > SLUG_MAX {
        // Keep the tail: `…/projects/onsager-ai/duhem` beats
        // `home-marvin-projects-…` for a human scanning the dir.
        let cut = slug
            .char_indices()
            .rev()
            .nth(SLUG_MAX - 1)
            .map(|(i, _)| i)
            .unwrap_or(0);
        slug = slug[cut..].trim_matches('-').to_string();
    }
    if slug.is_empty() {
        slug = "root".to_string();
    }

    format!("{slug}-{hash8}")
}

/// The default DB path for a working copy:
/// `<state_root>/projects/<slug>-<hash8>/duhem.db`.
pub fn project_db_path(workdir: &Path) -> Result<PathBuf, StoreError> {
    Ok(state_root()?
        .join("projects")
        .join(project_slug(workdir))
        .join("duhem.db"))
}

/// Where a serving dashboard advertises its base URL for the store at
/// `db_path` (#298): a `dashboard.addr` file next to the DB. The
/// dashboard writes `http://<host>:<port>` on bind and removes it on
/// shutdown; `duhem run` reads it (and probes the address, so a stale
/// file after a crash is harmless) to print a clickable live-run URL.
/// Keyed by DB location because that is the rendezvous the two
/// processes already share.
pub fn dashboard_addr_path(db_path: &Path) -> PathBuf {
    match db_path.parent() {
        Some(dir) => dir.join("dashboard.addr"),
        None => PathBuf::from("dashboard.addr"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_readable_and_hash_suffixed() {
        let dir = tempfile::tempdir().unwrap();
        let slug = project_slug(dir.path());
        // tail is an 8-hex hash, head is the sanitized path
        let (head, hash) = slug.rsplit_once('-').unwrap();
        assert_eq!(hash.len(), 8);
        assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(!head.is_empty());
        assert!(
            head.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn different_paths_get_different_slugs_same_path_is_stable() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_ne!(project_slug(a.path()), project_slug(b.path()));
        assert_eq!(project_slug(a.path()), project_slug(a.path()));
    }

    #[test]
    fn relative_and_canonical_spellings_key_identically() {
        let dir = tempfile::tempdir().unwrap();
        let via_dot = dir.path().join("sub").join("..");
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        assert_eq!(project_slug(dir.path()), project_slug(&via_dot));
    }

    #[test]
    fn long_paths_truncate_but_stay_unique_via_hash() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("x".repeat(200));
        std::fs::create_dir_all(&deep).unwrap();
        let slug = project_slug(&deep);
        // slug head capped, plus '-' + 8 hash chars
        assert!(slug.len() <= SLUG_MAX + 9, "slug too long: {}", slug.len());
    }

    #[test]
    fn duhem_home_overrides_state_root() {
        temp_env::with_var("DUHEM_HOME", Some("/custom/duhem/home"), || {
            assert_eq!(state_root().unwrap(), PathBuf::from("/custom/duhem/home"));
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn xdg_state_home_is_honored_on_linux() {
        temp_env::with_vars(
            [
                ("DUHEM_HOME", None::<&str>),
                ("XDG_STATE_HOME", Some("/tmp/xdg-state-test")),
            ],
            || {
                assert_eq!(
                    state_root().unwrap(),
                    PathBuf::from("/tmp/xdg-state-test/duhem")
                );
            },
        );
    }

    #[test]
    fn db_path_lands_under_projects_namespace() {
        let dir = tempfile::tempdir().unwrap();
        temp_env::with_var("DUHEM_HOME", Some(dir.path().as_os_str()), || {
            let db = project_db_path(dir.path()).unwrap();
            assert!(db.starts_with(dir.path().join("projects")));
            assert_eq!(db.file_name().unwrap(), "duhem.db");
        });
    }
}
