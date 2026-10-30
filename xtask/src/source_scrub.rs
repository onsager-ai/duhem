//! Forward-only private-name gate over the tracked working tree, not history.
use crate::private_vocab::{load_private_names, scan_private_line};
use anyhow::{Context, Result, bail};
use std::{path::Path, process::Command};

pub fn run(args: Vec<String>) -> Result<()> {
    if !args.is_empty() {
        bail!("source-scrub takes no arguments");
    }
    let names = load_private_names()?;
    let rules = names.iter().map(String::as_str).collect::<Vec<_>>();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")?;
    scan(root, &rules)
}

fn scan(root: &Path, rules: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .context("list tracked files")?;
    if !output.status.success() {
        bail!("git ls-files failed");
    }
    let mut files = 0;
    let mut hits = 0;
    for raw in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
    {
        let relative = std::str::from_utf8(raw).context("tracked path is not UTF-8")?;
        let path = root.join(relative);
        // Read symlinks as link targets, never follow a link outside the tree.
        let bytes = if path.is_symlink() {
            std::fs::read_link(&path)
                .context("read tracked symlink")?
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else {
            std::fs::read(&path).context("read tracked file")?
        };
        files += 1;
        for (line, text) in String::from_utf8_lossy(&bytes).lines().enumerate() {
            if !scan_private_line(text, rules).is_empty() {
                println!("  {relative}:{}", line + 1);
                hits += 1;
            }
        }
    }
    println!("source-scrub: {files} tracked files; {hits} matching locations (text omitted)");
    if hits > 0 {
        bail!("private-name locations in tracked files; remove them before publishing");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_tracked_files_including_hidden_and_ignores_untracked_files() {
        let root = std::env::temp_dir().join(format!("duhem-source-scrub-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            Command::new("git")
                .arg("init")
                .arg(&root)
                .output()
                .unwrap()
                .status
                .success()
        );
        std::fs::write(root.join(".hidden"), "safe\nRESTRICTED-EXAMPLE\n").unwrap();
        std::fs::write(root.join("untracked"), "restricted-example").unwrap();
        assert!(
            Command::new("git")
                .args(["add", ".hidden"])
                .current_dir(&root)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(scan(&root, &["restricted-example"]).is_err());
        std::fs::write(root.join(".hidden"), "ordinary content").unwrap();
        assert!(scan(&root, &["restricted-example"]).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
}
