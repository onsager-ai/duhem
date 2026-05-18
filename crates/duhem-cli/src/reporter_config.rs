//! Reporter-plugin discovery via TOML config files (spec on issue #34).
//!
//! Two locations:
//!
//! - **Repo** (`./.duhem.toml` in cwd or any ancestor) — takes
//!   precedence over user config.
//! - **User** (`~/.duhem/config.toml`) — fallback.
//!
//! Schema (both files):
//!
//! ```toml
//! [reporter.pretty]
//! command = ["duhem-reporter-pretty"]
//!
//! [reporter.junit]
//! command = ["python3", "-m", "duhem_reporter_junit"]
//! ```
//!
//! Plugin resolution order from `main.rs`: built-in match
//! (`default` / `quiet` / `json` always win) → repo config → user
//! config → error. Built-ins are not shadowable; if a config file
//! names one of them, the entry is ignored (the built-in implementation
//! is what runs).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One reporter plugin entry in a `[reporter.<name>]` table.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PluginEntry {
    /// `argv`-style command: program followed by zero or more args.
    /// Empty vec is rejected by the loader.
    pub command: Vec<String>,
}

/// Parsed reporter section from one TOML file.
#[derive(Debug, Default, Clone, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    reporter: BTreeMap<String, PluginEntry>,
}

/// Plugin registry resolved from repo + user config. Per the precedence
/// rule, a repo entry shadows a same-named user entry.
#[derive(Debug, Default, Clone)]
pub struct PluginRegistry {
    plugins: BTreeMap<String, PluginEntry>,
}

impl PluginRegistry {
    /// Load + merge repo and user config. Missing files are treated as
    /// empty (the common case — most authors don't have a config).
    /// Returns an error only if a present file fails to parse or
    /// contains a structurally invalid entry, so a typoed
    /// `command = ""` doesn't silently degrade to "plugin not found".
    pub fn load() -> Result<Self, String> {
        let mut user = ConfigFile::default();
        if let Some(p) = user_config_path()
            && p.exists()
        {
            user = parse_file(&p)?;
        }
        let mut repo = ConfigFile::default();
        if let Some(p) = find_repo_config(&std::env::current_dir().unwrap_or_default())
            && p.exists()
        {
            repo = parse_file(&p)?;
        }
        merge_layered(user.reporter, repo.reporter).map(|plugins| Self { plugins })
    }

    /// Build a registry from explicit entries. Used by tests and by
    /// callers that want to bypass filesystem lookup. Same emptiness
    /// rule as [`Self::load`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_entries(
        entries: impl IntoIterator<Item = (String, PluginEntry)>,
    ) -> Result<Self, String> {
        let mut plugins = BTreeMap::new();
        for (name, entry) in entries {
            if entry.command.is_empty() {
                return Err(format!(
                    "reporter `{name}`: `command` must be a non-empty argv list"
                ));
            }
            plugins.insert(name, entry);
        }
        Ok(Self { plugins })
    }

    pub fn get(&self, name: &str) -> Option<&PluginEntry> {
        self.plugins.get(name)
    }
}

/// Merge two reporter sections with repo overlaying user. Empty
/// command lists are rejected here (rather than at plugin-invocation
/// time) so a typoed config fails before the run starts. Exposed for
/// tests; production callers go through [`PluginRegistry::load`].
fn merge_layered(
    user: BTreeMap<String, PluginEntry>,
    repo: BTreeMap<String, PluginEntry>,
) -> Result<BTreeMap<String, PluginEntry>, String> {
    let mut out: BTreeMap<String, PluginEntry> = BTreeMap::new();
    for (name, entry) in user.into_iter().chain(repo) {
        if entry.command.is_empty() {
            return Err(format!(
                "reporter `{name}`: `command` must be a non-empty argv list"
            ));
        }
        out.insert(name, entry);
    }
    Ok(out)
}

fn parse_file(path: &Path) -> Result<ConfigFile, String> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| format!("reporter config {}: {e}", path.display()))?;
    toml::from_str::<ConfigFile>(&src)
        .map_err(|e| format!("reporter config {}: {e}", path.display()))
}

/// Walk from `start` up the directory chain looking for `.duhem.toml`.
/// Matches the discovery pattern of `.gitignore` / `Cargo.toml` —
/// authors can run `duhem` from a subdirectory of their checkout and
/// still pick up the repo-local config.
fn find_repo_config(start: &Path) -> Option<PathBuf> {
    let mut cur = Some(start.to_path_buf());
    while let Some(dir) = cur {
        let candidate = dir.join(".duhem.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        cur = dir.parent().map(Path::to_path_buf);
    }
    None
}

/// `~/.duhem/config.toml` location, or `None` if the home directory
/// can't be determined (uncommon — the OS would have to refuse
/// `$HOME`).
fn user_config_path() -> Option<PathBuf> {
    // We intentionally avoid the `dirs` crate to keep `duhem-cli`'s
    // dependency surface tight. The `$HOME` env var is honored on
    // Unix; on Windows we'd add `%USERPROFILE%`, but Phase 0 is
    // Linux/macOS only.
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".duhem").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_entries_rejects_empty_command() {
        let err =
            PluginRegistry::from_entries([("broken".to_string(), PluginEntry { command: vec![] })])
                .unwrap_err();
        assert!(
            err.contains("broken") && err.contains("non-empty"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_file_accepts_well_formed_toml() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".duhem.toml");
        std::fs::write(
            &p,
            r#"
[reporter.pretty]
command = ["duhem-reporter-pretty"]

[reporter.junit]
command = ["python3", "-m", "duhem_reporter_junit"]
"#,
        )
        .unwrap();
        let cfg = parse_file(&p).unwrap();
        assert_eq!(
            cfg.reporter["pretty"].command,
            vec!["duhem-reporter-pretty".to_string()]
        );
        assert_eq!(cfg.reporter["junit"].command.len(), 3);
    }

    #[test]
    fn parse_file_surfaces_toml_errors_with_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".duhem.toml");
        std::fs::write(&p, "this is = not = toml\n").unwrap();
        let err = parse_file(&p).unwrap_err();
        assert!(err.contains(".duhem.toml"), "error should name path: {err}");
    }

    #[test]
    fn find_repo_config_walks_upward() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        let cfg = dir.path().join(".duhem.toml");
        std::fs::write(&cfg, "[reporter]\n").unwrap();
        assert_eq!(find_repo_config(&nested).as_deref(), Some(cfg.as_path()));
    }

    #[test]
    fn find_repo_config_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_repo_config(dir.path()).is_none());
    }

    #[test]
    fn repo_overrides_user_on_same_plugin_name() {
        // Spec on #34 Test § "Repo config overrides user config when
        // both define the same `<name>`". Build two layered configs
        // with overlapping name `pretty` and confirm the repo argv wins.
        let user_entry = PluginEntry {
            command: vec!["user-pretty".to_string()],
        };
        let repo_entry = PluginEntry {
            command: vec!["repo-pretty".to_string(), "--from-repo".to_string()],
        };
        let mut user = BTreeMap::new();
        user.insert("pretty".to_string(), user_entry);
        let mut repo = BTreeMap::new();
        repo.insert("pretty".to_string(), repo_entry.clone());
        let merged = merge_layered(user, repo).unwrap();
        assert_eq!(merged["pretty"], repo_entry);
    }

    #[test]
    fn repo_only_entry_carries_through_when_user_absent() {
        // Inverse half of the precedence test: user empty, repo
        // defines `junit` — `junit` must reach the merged map.
        let repo_entry = PluginEntry {
            command: vec!["python3".to_string(), "-m".to_string(), "junit".to_string()],
        };
        let mut repo = BTreeMap::new();
        repo.insert("junit".to_string(), repo_entry.clone());
        let merged = merge_layered(BTreeMap::new(), repo).unwrap();
        assert_eq!(merged["junit"], repo_entry);
    }
}
