#[path = "src/build_version.rs"]
mod duhem_evidence_build_version;

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=DUHEM_BUILD_COMMIT");

    let commit = std::env::var("DUHEM_BUILD_COMMIT")
        .ok()
        .filter(|commit| !commit.is_empty())
        .or_else(git_commit);
    let version = duhem_evidence_build_version::format_build_version(
        env!("CARGO_PKG_VERSION"),
        commit.as_deref(),
    );
    println!("cargo:rustc-env=DUHEM_BUILD_VERSION={version}");
}

fn git_commit() -> Option<String> {
    let head_path = git_path("HEAD")?;
    println!("cargo:rerun-if-changed={}", head_path.display());
    if let Ok(head) = std::fs::read_to_string(&head_path)
        && let Some(reference) = head.strip_prefix("ref: ").map(str::trim)
        && let Some(reference_path) = git_path(reference)
    {
        println!("cargo:rerun-if-changed={}", reference_path.display());
    }

    Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|commit| commit.trim().to_owned())
        .filter(|commit| !commit.is_empty())
}

fn git_path(path: &str) -> Option<PathBuf> {
    Command::new("git")
        .args(["rev-parse", "--git-path", path])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|path| PathBuf::from(path.trim()))
        .filter(|path| !path.as_os_str().is_empty())
}
