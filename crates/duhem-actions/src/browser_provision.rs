//! Browser auto-provisioning for `duhem run` (#295).
//!
//! When a `ui/*` check needs a browser and the sidecar can't launch one
//! (no Chromium, and the discovery fallback from #105 found nothing), the
//! runtime installs it once — binary only, never `--with-deps`, so system
//! libraries and sudo stay an explicit `duhem browser install` choice —
//! then retries. A stale pin that predates the host distro is worked
//! around by forcing the nearest supported-LTS build. Opt out with
//! `DUHEM_NO_BROWSER_INSTALL`.
//!
//! Split out of `browser.rs` to keep that driver module under the
//! file-token budget; [`crate::browser::RunBrowser::launch`] is the sole
//! caller.

use tokio::process::Command;

use crate::browser::sidecar_dir;

/// Playwright env that forces a specific host-platform build. When a
/// pinned Playwright is older than the host distro, `playwright install`
/// refuses (`does not support chromium on <distro>`) even though the
/// prebuilt `ubuntu24.04` Chromium runs fine on newer releases. Forcing
/// this build is the install fallback for a not-yet-tabled LTS (#295).
pub const HOST_PLATFORM_OVERRIDE_ENV: &str = "PLAYWRIGHT_HOST_PLATFORM_OVERRIDE";
/// The nearest supported-LTS build to force via [`HOST_PLATFORM_OVERRIDE_ENV`].
pub const HOST_PLATFORM_OVERRIDE_VALUE: &str = "ubuntu24.04-x64";

/// Recognize the Playwright "no prebuilt browser for this distro" refusal
/// that [`HOST_PLATFORM_OVERRIDE_ENV`] works around (#295).
pub(crate) fn is_unsupported_distro_error(s: &str) -> bool {
    s.to_lowercase().contains("does not support chromium")
}

/// Whether a launch error means the Chromium binary (or the sidecar's npm
/// deps) is absent *and* the sidecar's discovery fallback (#105) found
/// nothing — the case `duhem run` auto-provisions (#295). Matches the raw
/// sidecar/Playwright text; a superset of `humanize_launch_error`'s
/// browser-missing / deps-missing branches.
pub(crate) fn is_missing_browser_error(s: &str) -> bool {
    let l = s.to_lowercase();
    l.contains("executable doesn't exist")
        || l.contains("no existing chromium was found")
        || l.contains("install missing dependencies")
        || l.contains("looks like playwright")
        || l.contains("cannot find package 'playwright'")
        || l.contains("err_module_not_found")
        || l.contains("dependencies or the chromium browser are likely not installed")
}

/// The `DUHEM_NO_BROWSER_INSTALL` truthiness rule: `1` / `true`
/// (case-insensitive, trimmed) opts out of `duhem run`'s auto-provision.
/// Pure (no env access) so it is unit-testable, mirroring `env_headed`.
pub(crate) fn no_install_truthy(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true")
}

/// Whether auto-provision is opted out via `DUHEM_NO_BROWSER_INSTALL`.
pub(crate) fn auto_install_disabled() -> bool {
    std::env::var("DUHEM_NO_BROWSER_INSTALL")
        .ok()
        .is_some_and(|v| no_install_truthy(&v))
}

/// Combined-output result of a provisioning subprocess. `ok` is the exit
/// status; `text` is stdout+stderr for distro-refusal classification.
struct CmdOut {
    ok: bool,
    text: String,
}

/// Run a command in `dir` capturing combined stdout+stderr, with optional
/// extra env. `Err` is a spawn failure; `Ok(CmdOut{ok:false,..})` is a
/// non-zero exit — the caller inspects `text` to decide the retry.
async fn run_capture(
    program: &str,
    args: &[&str],
    dir: &std::path::Path,
    envs: &[(&str, &str)],
) -> Result<CmdOut, String> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .await
        .map_err(|e| format!("failed to run `{program}`: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(CmdOut {
        ok: out.status.success(),
        text,
    })
}

/// Best-effort cross-process lock over the sidecar dir, so concurrent
/// `duhem run`s don't race the one-time provision. Held for the install;
/// released on drop (the file closes). Best-effort: if the lock can't be
/// taken, provisioning proceeds anyway — `playwright install` is
/// idempotent and does its own download locking.
struct ProvisionLock {
    _file: std::fs::File,
}

impl ProvisionLock {
    fn acquire(dir: &std::path::Path) -> Option<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(dir.join(".duhem-install.lock"))
            .ok()?;
        // Blocks until the exclusive advisory lock is free (std, Rust 1.89+).
        file.lock().ok()?;
        Some(Self { _file: file })
    }
}

/// Provision the Chromium binary (and the sidecar's npm deps if absent)
/// for `duhem run`, so a fresh host drives the UI without a separate
/// `duhem browser install` (#295). Binary-only — never `--with-deps`, so
/// system libraries + sudo stay an explicit `browser install` choice.
/// Returns `Ok` on success; `Err(reason)` on any failure, so the caller
/// falls back to the actionable launch error.
pub(crate) async fn provision_browser() -> Result<(), String> {
    let dir = sidecar_dir();
    if !dir.join("index.mjs").exists() {
        return Err(format!(
            "sidecar not materialized at {} — run `duhem browser install`",
            dir.display()
        ));
    }
    eprintln!(
        "[duhem] Chromium not found — installing it once into {} (~110 MiB). Set DUHEM_NO_BROWSER_INSTALL=1 to skip and manage the browser yourself.",
        dir.display()
    );
    let _lock = ProvisionLock::acquire(&dir);

    // 1. Sidecar npm deps (playwright), only if absent.
    if !dir.join("node_modules").join("playwright").exists() {
        let out = run_capture("npm", &["ci"], &dir, &[]).await?;
        if !out.ok {
            let retry = run_capture("npm", &["install"], &dir, &[]).await?;
            if !retry.ok {
                return Err(format!("npm install failed:\n{}", retry.text.trim()));
            }
        }
    }

    // 2. Chromium binary — binary only. Retry once forcing the nearest
    //    supported-LTS build when Playwright refuses this distro (#295).
    let args = ["--yes", "playwright", "install", "chromium"];
    let out = run_capture("npx", &args, &dir, &[]).await?;
    if out.ok {
        return Ok(());
    }
    if is_unsupported_distro_error(&out.text) {
        eprintln!(
            "[duhem] Playwright ships no prebuilt Chromium for this OS; retrying with {HOST_PLATFORM_OVERRIDE_ENV}={HOST_PLATFORM_OVERRIDE_VALUE}…"
        );
        let retry = run_capture(
            "npx",
            &args,
            &dir,
            &[(HOST_PLATFORM_OVERRIDE_ENV, HOST_PLATFORM_OVERRIDE_VALUE)],
        )
        .await?;
        if retry.ok {
            return Ok(());
        }
        return Err(format!(
            "playwright install failed even with {HOST_PLATFORM_OVERRIDE_ENV}:\n{}",
            retry.text.trim()
        ));
    }
    Err(format!("playwright install failed:\n{}", out.text.trim()))
}

#[cfg(test)]
mod tests {
    use super::{is_missing_browser_error, is_unsupported_distro_error, no_install_truthy};

    #[test]
    fn missing_browser_triggers_auto_provision() {
        // The raw sidecar/Playwright strings that mean "no browser, and
        // discovery found nothing" — each must arm auto-provision (#295).
        for raw in [
            "browserType.launch: Executable doesn't exist at /…/chrome-headless-shell",
            "… — and no existing Chromium was found to fall back to. Install one …",
            "the Playwright sidecar exited before responding — its dependencies or the Chromium browser are likely not installed.",
            "Cannot find package 'playwright' imported from …",
        ] {
            assert!(is_missing_browser_error(raw), "should match: {raw}");
        }
        // A live-site failure must NOT trigger an install.
        assert!(!is_missing_browser_error(
            "Timeout 5000ms exceeded waiting for selector \"#hi\""
        ));
    }

    #[test]
    fn unsupported_distro_is_recognized() {
        assert!(is_unsupported_distro_error(
            "Error: ERROR: Playwright does not support chromium on ubuntu26.04-x64"
        ));
        assert!(!is_unsupported_distro_error(
            "Download failed: connection reset"
        ));
    }

    #[test]
    fn no_install_optout_truthiness() {
        for on in ["1", "true", "TRUE", " true "] {
            assert!(no_install_truthy(on), "should opt out: {on:?}");
        }
        for off in ["", "0", "false", "no", "yes", "2"] {
            assert!(!no_install_truthy(off), "should not opt out: {off:?}");
        }
    }
}
