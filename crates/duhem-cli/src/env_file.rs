//! Small, explicit `.env` sourcing seam for `duhem run`.
//!
//! A verification tool should not silently disagree with the convention its
//! users expect. This parser therefore implements a small documented subset
//! that is easier to reason about than a large implicit one; replacing it with
//! a maintained crate later is contained behind this module.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) fn source_for_run(
    path: Option<&Path>,
    manifest_override: Option<&Path>,
    explicit_env: Option<&Path>,
    disabled: bool,
) -> Result<(), String> {
    if disabled {
        return Ok(());
    }
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot determine current directory: {e}"))?;
    let env_path = if let Some(explicit) = explicit_env {
        Some(if explicit.is_absolute() {
            explicit.to_owned()
        } else {
            cwd.join(explicit)
        })
    } else {
        discover_env_file(path, manifest_override, &cwd)?
    };
    let Some(env_path) = env_path else {
        return Ok(());
    };
    let source = std::fs::read_to_string(&env_path)
        .map_err(|e| format!("read environment file {}: {e}", env_path.display()))?;
    let values = parse(&source).map_err(|e| format!("{}: {e}", env_path.display()))?;
    for (key, value) in values {
        if std::env::var_os(&key).is_none() {
            // SAFETY: dispatch calls this before constructing the Tokio runtime
            // or spawning any worker/sidecar threads. No concurrent environment
            // access exists at this point in the CLI process.
            unsafe { std::env::set_var(key, value) };
        }
    }
    Ok(())
}

fn discover_env_file(
    path: Option<&Path>,
    manifest_override: Option<&Path>,
    cwd: &Path,
) -> Result<Option<PathBuf>, String> {
    let target = match manifest_override {
        Some(p) => {
            if p.is_absolute() {
                p.to_owned()
            } else {
                cwd.join(p)
            }
        }
        None => duhem_schema::discover(path, cwd)
            .map_err(|e| format!("[schema v{}] {e}", duhem_schema::SCHEMA_VERSION))?,
    };
    let target = if target.is_absolute() {
        target
    } else {
        cwd.join(target)
    };
    let start = if manifest_override.is_some() {
        target.parent().map(Path::to_owned)
    } else {
        manifest_dir_for_target(&target, cwd)
    }
    .unwrap_or_else(|| cwd.to_owned());
    Ok(walk_for_dotenv(&start))
}

fn manifest_dir_for_target(target: &Path, cwd: &Path) -> Option<PathBuf> {
    let parent = if target.is_dir() {
        target
    } else {
        target.parent()?
    };
    const MANIFEST_NAMES: [&str; 4] = ["duhem.yml", "duhem.yaml", ".duhem.yml", ".duhem.yaml"];
    if target
        .file_name()
        .is_some_and(|n| MANIFEST_NAMES.iter().any(|c| n == *c))
    {
        return Some(parent.to_owned());
    }
    duhem_schema::discover(Some(parent), cwd)
        .ok()
        .and_then(|p| p.parent().map(Path::to_owned))
}

fn walk_for_dotenv(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(current) = dir {
        let candidate = current.join(".env");
        if candidate.is_file() {
            return Some(candidate);
        }
        if current.join(".git").exists() {
            break;
        }
        dir = current.parent();
    }
    None
}

fn parse(source: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (index, raw) in source.lines().enumerate() {
        let line_no = index + 1;
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("export ") {
            line = rest.trim_start();
        }
        let (key, raw_value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {line_no}: expected KEY=VALUE"))?;
        let key = key.trim();
        if key.is_empty()
            || !key
                .bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
        {
            return Err(format!("line {line_no}: invalid environment key `{key}`"));
        }
        let value = parse_value(raw_value, line_no)?;
        if value.contains('\0') {
            return Err(format!(
                "line {line_no}: NUL bytes are not valid in environment values"
            ));
        }
        if value.contains("${") {
            return Err(format!(
                "line {line_no}: variable interpolation is not supported"
            ));
        }
        out.insert(key.to_owned(), value);
    }
    Ok(out)
}

fn parse_value(raw: &str, line_no: usize) -> Result<String, String> {
    let value = raw.trim();
    if let Some(inner) = value.strip_prefix('"') {
        let inner = inner.strip_suffix('"').ok_or_else(|| {
            format!(
                "line {line_no}: multi-line or unterminated double-quoted value is not supported"
            )
        })?;
        let mut parsed = String::new();
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('n') => parsed.push('\n'),
                    Some('t') => parsed.push('\t'),
                    Some(next) => {
                        parsed.push('\\');
                        parsed.push(next);
                    }
                    None => parsed.push('\\'),
                }
            } else {
                parsed.push(ch);
            }
        }
        Ok(parsed)
    } else if let Some(inner) = value.strip_prefix('\'') {
        inner.strip_suffix('\'').map(str::to_owned).ok_or_else(|| {
            format!(
                "line {line_no}: multi-line or unterminated single-quoted value is not supported"
            )
        })
    } else {
        Ok(value
            .split_once(" #")
            .map_or(value, |(v, _)| v)
            .trim()
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_subset() {
        let values =
            parse("# note\n export A = plain  # tail\nB=' literal '\nC=\"line\\ncol\\tend\"\n")
                .unwrap();
        assert_eq!(values["A"], "plain");
        assert_eq!(values["B"], " literal ");
        assert_eq!(values["C"], "line\ncol\tend");
    }

    #[test]
    fn rejects_interpolation_and_unterminated_quotes() {
        assert!(parse("A=${OTHER}\n").unwrap_err().contains("interpolation"));
        assert!(
            parse("A='first\nsecond\n")
                .unwrap_err()
                .contains("multi-line")
        );
    }

    #[test]
    fn discovery_stops_at_git_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), "A=one\n").unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let child = tmp.path().join("a/b");
        std::fs::create_dir_all(&child).unwrap();
        assert_eq!(walk_for_dotenv(&child), Some(tmp.path().join(".env")));
    }
}
