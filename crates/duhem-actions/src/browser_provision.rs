//! Browser install mechanics, shared by `duhem run`'s auto-provision and
//! the explicit `duhem browser install` command (#295, unified #505).
//!
//! When a `ui/*` check needs a browser and the sidecar can't launch one
//! (no Chromium, and the discovery fallback from #105 found nothing), the
//! runtime installs it once — binary only, never `--with-deps`, so system
//! libraries and sudo stay an explicit `duhem browser install` choice —
//! then retries. A stale pin that predates the host distro is worked
//! around by forcing the nearest supported-LTS build. Opt out with
//! `DUHEM_NO_BROWSER_INSTALL`.
//!
//! [`install_browser`] holds the mechanics both callers need — the npm
//! deps step, `npx playwright install`, the distro-refusal retry, and the
//! cross-process lock — parameterized by an [`InstallPolicy`] so each
//! caller keeps its own rules (`--with-deps`, the deps-usable skip) and
//! its own voice (terse `[duhem]` stderr lines vs. friendly stdout
//! progress) without duplicating the retry logic. [`provision_browser`]
//! is the auto-provision caller; `duhem-cli`'s `browser_cmd` is the other.
//!
//! Split out of `browser.rs` to keep that driver module under the
//! file-token budget; [`crate::browser::RunBrowser::launch`] is the sole
//! in-crate caller.

use std::io::Write;

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
///
/// `stream` is [`InstallPolicy::stream_output`]: `false` (auto-provision)
/// captures silently, so npm/npx chatter never lands in the middle of an
/// unrelated `duhem run`'s own output; `true` (the CLI) additionally
/// echoes each line to our own stdout/stderr as it arrives, so a user who
/// typed `duhem browser install` watches Playwright's live download
/// progress instead of staring at nothing for the minutes the ~110 MiB
/// Chromium download can take. Either way, `text` — the full
/// stdout+stderr, accumulated — is what the shared retry logic below
/// classifies a distro refusal against.
async fn run_capture(
    program: &str,
    args: &[&str],
    dir: &std::path::Path,
    envs: &[(&str, &str)],
    stream: bool,
) -> Result<CmdOut, String> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir);
    for (k, v) in envs {
        cmd.env(k, v);
    }

    if !stream {
        let out = cmd
            .output()
            .await
            .map_err(|e| format!("failed to run `{program}`: {e}"))?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        return Ok(CmdOut {
            ok: out.status.success(),
            text,
        });
    }

    // Streaming (tee) path: pipe both streams and drain them
    // CONCURRENTLY via `tokio::join!`. Draining one to EOF before
    // starting the other would deadlock once the child fills the OS
    // pipe buffer on the *other* stream (64 KiB on Linux): the child
    // blocks writing to it, nothing is reading it, and we're stuck
    // waiting for the first stream to reach EOF — which it never will,
    // because the child never gets back to producing the rest of it.
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to run `{program}`: {e}"))?;
    let stdout = child.stdout.take().expect("stdout piped above");
    let stderr = child.stderr.take().expect("stderr piped above");

    let (stdout_text, stderr_text) =
        tokio::join!(tee_lines(stdout, false), tee_lines(stderr, true));

    let status = child
        .wait()
        .await
        .map_err(|e| format!("failed to wait on `{program}`: {e}"))?;

    let mut text = stdout_text;
    text.push_str(&stderr_text);
    Ok(CmdOut {
        ok: status.success(),
        text,
    })
}

/// Drain one child pipe line-by-line for the streaming path: echo each
/// line to our own stdout (`to_stderr: false`) or stderr (`true`) as it
/// arrives, while also accumulating the full text (newline-joined) —
/// [`run_capture`]'s caller still needs the combined text even when it
/// wants the output echoed live. Interleaving between the stdout and
/// stderr copies is not guaranteed (they're two independent halves of a
/// `tokio::join!`, not a single ordered stream); that's fine for a human
/// watching a terminal and not a property either side asserts on.
///
/// Deliberately byte-oriented (`read_until(b'\n', ..)` + a lossy UTF-8
/// decode of each line), not `AsyncBufReadExt::lines()`: `lines()`
/// yields `Err` — indistinguishable here from EOF under a `while let
/// Ok(Some(..))` loop — the moment a line isn't valid UTF-8, which would
/// stop draining this pipe on one stray byte, stall the child once the
/// *other* pipe's OS buffer then fills, and hang `child.wait()`; that
/// defeats the very concurrent-drain this function exists to provide. A
/// "skip only `InvalidData` and keep going" loop around `lines()` would
/// dodge that, but only by relying on `read_line` having already
/// consumed the bad line's bytes before failing UTF-8 validation — an
/// implementation detail of `lines()`, not part of its documented
/// contract, so a future change there could reintroduce the same hang.
/// `read_until` has no such failure mode by construction: invalid UTF-8
/// is never an `Err` to classify, so there's nothing to skip. It's also
/// more correct on its own terms — skipping a line, even on the
/// "impossible in practice" theory, would *drop* it, and
/// `is_unsupported_distro_error` classifies against this accumulated
/// text; if the dropped line were the distro-refusal message with one
/// stray byte in it, the retry would silently not fire. Lossy decoding
/// keeps the line (with U+FFFD standing in for the bad bytes) so both
/// the classifier and the user watching still see it.
async fn tee_lines(stream: impl tokio::io::AsyncRead + Unpin, to_stderr: bool) -> String {
    use tokio::io::AsyncBufReadExt;

    let mut reader = tokio::io::BufReader::new(stream);
    let mut raw = Vec::new();
    let mut text = String::new();
    loop {
        raw.clear();
        match reader.read_until(b'\n', &mut raw).await {
            Ok(0) => break,  // EOF
            Err(_) => break, // genuine I/O error on this pipe
            Ok(_) => {
                // `read_until` includes the delimiter; strip it (and a
                // preceding `\r`, for CRLF output) since `println!` /
                // `eprintln!` add their own newline.
                let mut line = raw.as_slice();
                if line.last() == Some(&b'\n') {
                    line = &line[..line.len() - 1];
                }
                if line.last() == Some(&b'\r') {
                    line = &line[..line.len() - 1];
                }
                let line = String::from_utf8_lossy(line);
                if to_stderr {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                }
                text.push_str(&line);
                text.push('\n');
            }
        }
    }
    text
}

/// Best-effort cross-process lock over the sidecar dir, so concurrent
/// `duhem run`s don't race the one-time provision. Held for the install;
/// released on drop (the file closes). Best-effort: if the lock can't be
/// taken, provisioning proceeds anyway — `playwright install` is
/// idempotent and does its own download locking.
struct ProvisionLock {
    _file: std::fs::File,
}

const INSTALL_LOCK: &str = ".duhem-install.lock";

/// A Playwright directory can exist after an interrupted `npm ci` while
/// containing none of the package. Treat only its parseable package manifest
/// as an installed dependency tree.
fn sidecar_deps_usable(dir: &std::path::Path) -> bool {
    let manifest = dir.join("node_modules/playwright/package.json");
    std::fs::read(manifest)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|json| json.get("name")?.as_str().map(str::to_owned))
        .is_some_and(|name| name == "playwright")
}

fn recorded_lock_owner(dir: &std::path::Path) -> Option<u32> {
    std::fs::read_to_string(dir.join(INSTALL_LOCK))
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
fn process_is_running(pid: u32) -> bool {
    std::path::Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(not(target_os = "linux"))]
fn process_is_running(_pid: u32) -> bool {
    // The advisory lock remains authoritative on platforms without procfs.
    true
}

fn recorded_lock_is_stale(dir: &std::path::Path) -> bool {
    recorded_lock_owner(dir).is_some_and(|pid| !process_is_running(pid))
}

impl ProvisionLock {
    fn acquire(dir: &std::path::Path) -> Option<Self> {
        if recorded_lock_is_stale(dir) {
            eprintln!("[duhem] recovering stale Playwright sidecar install lock");
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(dir.join(INSTALL_LOCK))
            .ok()?;
        // Blocks until the exclusive advisory lock is free (std, Rust 1.89+).
        file.lock().ok()?;
        file.set_len(0).ok()?;
        writeln!(file, "{}", std::process::id()).ok()?;
        file.sync_data().ok()?;
        Some(Self { _file: file })
    }
}

/// Progress-reporting hook for [`install_browser`], so the two callers
/// keep their own voice without the shared mechanics caring which one is
/// driving: `duhem run`'s auto-provision prints terse `[duhem]` lines to
/// stderr (see [`ProvisionProgress`]); `duhem browser install` prints
/// friendly `→ …` progress to stdout (`duhem-cli::browser_cmd`). Every
/// method is a no-op by default so a caller only overrides what it wants
/// to say.
pub trait InstallProgress {
    /// About to run the npm deps step (`npm ci`, falling back to
    /// `npm install`).
    fn npm_start(&self) {}
    /// `npm ci` failed; retrying with `npm install`.
    fn npm_retry(&self) {}
    /// About to run `npx playwright install [--with-deps] chromium`.
    fn chromium_start(&self, _with_deps: bool) {}
    /// The Chromium install was refused for this distro; retrying with
    /// [`HOST_PLATFORM_OVERRIDE_ENV`] forcing the nearest supported LTS.
    fn distro_retry(&self) {}
}

/// [`InstallProgress`] for `duhem run`'s auto-provision: silent except
/// for the distro-refusal retry, which is worth a line since it changes
/// what gets installed.
struct ProvisionProgress;

impl InstallProgress for ProvisionProgress {
    fn distro_retry(&self) {
        eprintln!(
            "[duhem] Playwright ships no prebuilt Chromium for this OS; retrying with {HOST_PLATFORM_OVERRIDE_ENV}={HOST_PLATFORM_OVERRIDE_VALUE}…"
        );
    }
}

/// The policy inputs that legitimately differ between `duhem run`'s
/// auto-provision and the explicit `duhem browser install` — everything
/// else (the retry condition, the lock) is shared mechanics in
/// [`install_browser`] and not a policy choice.
pub struct InstallPolicy<'a> {
    /// Passed through to `npx playwright install`. Auto-provision always
    /// passes `false` — system libraries + sudo stay an explicit
    /// `browser install` choice; only the CLI ever sets this `true`.
    pub with_deps: bool,
    /// Skip the npm deps step when [`sidecar_deps_usable`] already holds.
    /// Auto-provision sets this so a `duhem run` retry stays fast; the
    /// explicit command always reinstalls (it documents itself as
    /// "idempotent; safe to re-run").
    pub skip_npm_when_usable: bool,
    /// Tee each install subprocess's output to our own stdout/stderr as
    /// it arrives (see [`run_capture`]), on top of capturing it. The
    /// explicit command sets this `true` — a user who typed `duhem
    /// browser install` should watch the ~110 MiB Chromium download live,
    /// not stare at silence for minutes. Auto-provision sets this
    /// `false`: it fires in the middle of an unrelated `duhem run`, where
    /// dumping npm/npx chatter into that run's own output would be noise
    /// at best and misattributed at worst.
    pub stream_output: bool,
    /// Where to report progress (see [`InstallProgress`]).
    pub progress: &'a dyn InstallProgress,
}

/// Whether the npm deps step should run, given the policy's
/// deps-usable skip and whether the sidecar's deps are actually usable.
/// Pure so the skip policy (auto-provision skips when usable; the
/// explicit command always runs) is unit-testable without touching a
/// filesystem or a subprocess.
fn need_npm_step(skip_when_usable: bool, deps_usable: bool) -> bool {
    !skip_when_usable || !deps_usable
}

/// The `npx playwright install [--with-deps] chromium` argv. Pure so the
/// `--with-deps` policy (auto-provision never; the explicit command iff
/// requested) is unit-testable without spawning anything.
fn chromium_install_args(with_deps: bool) -> Vec<&'static str> {
    let mut args = vec!["--yes", "playwright", "install"];
    if with_deps {
        args.push("--with-deps");
    }
    args.push("chromium");
    args
}

/// Install the Chromium binary (and the sidecar's npm deps, per
/// `policy`) into `dir`. Shared mechanics for `duhem run`'s
/// auto-provision and `duhem browser install` (#505): the npm step, the
/// `npx playwright install` retry gated on
/// [`is_unsupported_distro_error`] (never on any other failure — an
/// unconditional retry would mask real errors and double the wait), and
/// a best-effort cross-process [`ProvisionLock`] so the two don't race
/// each other. Returns `Ok` on success; `Err(reason)` — with the
/// captured subprocess output folded in — on any failure, so a caller
/// still sees what happened even when `stream_output` kept it silent
/// while running.
pub async fn install_browser(
    dir: &std::path::Path,
    policy: &InstallPolicy<'_>,
) -> Result<(), String> {
    let _lock = ProvisionLock::acquire(dir);

    // 1. Sidecar npm deps (playwright), reinstalling a partial tree left by
    //    an interrupted npm invocation.
    if need_npm_step(policy.skip_npm_when_usable, sidecar_deps_usable(dir)) {
        policy.progress.npm_start();
        let out = run_capture("npm", &["ci"], dir, &[], policy.stream_output).await?;
        if !out.ok {
            policy.progress.npm_retry();
            let retry = run_capture("npm", &["install"], dir, &[], policy.stream_output).await?;
            if !retry.ok {
                return Err(format!("npm install failed:\n{}", retry.text.trim()));
            }
        }
    }

    // 2. Chromium binary. Retry once forcing the nearest supported-LTS
    //    build when Playwright refuses this distro (#295) — and only
    //    then; any other failure (network, disk, …) is returned as-is.
    let args = chromium_install_args(policy.with_deps);

    policy.progress.chromium_start(policy.with_deps);
    let out = run_capture("npx", &args, dir, &[], policy.stream_output).await?;
    if out.ok {
        return Ok(());
    }
    if is_unsupported_distro_error(&out.text) {
        policy.progress.distro_retry();
        let retry = run_capture(
            "npx",
            &args,
            dir,
            &[(HOST_PLATFORM_OVERRIDE_ENV, HOST_PLATFORM_OVERRIDE_VALUE)],
            policy.stream_output,
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
    install_browser(
        &dir,
        &InstallPolicy {
            with_deps: false,
            skip_npm_when_usable: true,
            stream_output: false,
            progress: &ProvisionProgress,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        INSTALL_LOCK, ProvisionLock, chromium_install_args, is_missing_browser_error,
        is_unsupported_distro_error, need_npm_step, no_install_truthy, recorded_lock_is_stale,
        run_capture, sidecar_deps_usable,
    };

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "duhem-browser-provision-{label}-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

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

    #[test]
    fn partial_tree_with_stale_lock_requires_reinstall() {
        let dir = temp_dir("partial-deps");
        std::fs::create_dir_all(dir.join("node_modules/playwright")).unwrap();
        std::fs::write(dir.join(INSTALL_LOCK), format!("{}\n", u32::MAX)).unwrap();
        assert!(!sidecar_deps_usable(&dir));
        #[cfg(target_os = "linux")]
        assert!(recorded_lock_is_stale(&dir));

        std::fs::write(
            dir.join("node_modules/playwright/package.json"),
            r#"{"name":"playwright"}"#,
        )
        .unwrap();
        assert!(sidecar_deps_usable(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn with_deps_is_a_policy_choice_not_a_default() {
        // Auto-provision's fixed choice (`InstallPolicy { with_deps: false,
        // .. }` in `provision_browser`): binary-only, never `--with-deps`.
        assert_eq!(
            chromium_install_args(false),
            vec!["--yes", "playwright", "install", "chromium"]
        );
        // The explicit `duhem browser install --with-deps` choice.
        assert_eq!(
            chromium_install_args(true),
            vec!["--yes", "playwright", "install", "--with-deps", "chromium"]
        );
    }

    #[test]
    fn deps_usable_skip_is_a_policy_choice() {
        // Auto-provision (`skip_npm_when_usable: true`) skips npm only
        // when the existing tree is usable — it must stay fast on retry.
        assert!(!need_npm_step(true, true));
        assert!(need_npm_step(true, false));
        // The explicit command (`skip_npm_when_usable: false`) always
        // reinstalls, regardless of whether the tree looks usable — it
        // documents itself as "idempotent; safe to re-run".
        assert!(need_npm_step(false, true));
        assert!(need_npm_step(false, false));
    }

    #[test]
    fn lock_acquire_excludes_a_concurrent_holder() {
        // `ProvisionLock::acquire` (browser_provision.rs) takes
        // `std::fs::File::lock()`, which on Unix is `flock(2)` — a lock
        // on the *open file description*, not the process or the fd
        // number. Two independent `open()`s of the same path, even from
        // two threads in this one process, get two open file
        // descriptions and therefore genuinely contend, the same way two
        // separate `duhem run`/`duhem browser install` processes racing
        // the same sidecar dir would. Confirmed empirically before
        // writing this test (a throwaway two-thread program showed B
        // blocked while A held the lock, and only proceeded once A
        // dropped it) rather than assumed from docs, because the other
        // common flavor of advisory lock — POSIX `fcntl` byte-range
        // locks — is associated with the *process*, not the open file
        // description, and would NOT self-block two opens from one
        // process; a test written against that assumption would pass
        // for the wrong reason on a platform using that flavor.
        //
        // Honesty notes, per the brief:
        // 1. The negative assertion below ("B did not acquire while A
        //    held") is a bounded-wait timeout, not a proof of exclusion.
        //    A sufficiently pathological scheduler could in principle
        //    delay B past the window even without the lock; this test
        //    accepts that as the standard trade-off for testing blocking
        //    behavior without a flaky indefinite wait.
        // 2. This is two threads, one process — a proxy for the
        //    cross-process race the lock exists for, via the same
        //    kernel primitive (flock is already cross-process by
        //    design; a same-process exercise of it does not prove
        //    cross-process contention on every platform/filesystem,
        //    e.g. network filesystems where flock is unreliable). A
        //    stronger version would re-exec the test binary via
        //    `std::env::current_exe` behind an env flag to get two real
        //    processes; that was judged not worth the added complexity
        //    for a lock that is documented as best-effort.
        let dir = temp_dir("lock-mutex");

        let (a_holds_tx, a_holds_rx) = std::sync::mpsc::channel::<()>();
        let (release_a_tx, release_a_rx) = std::sync::mpsc::channel::<()>();
        let (b_acquired_tx, b_acquired_rx) = std::sync::mpsc::channel::<()>();

        let dir_a = dir.clone();
        let thread_a = std::thread::spawn(move || {
            let _lock = ProvisionLock::acquire(&dir_a).expect("thread A acquires the lock");
            a_holds_tx.send(()).unwrap();
            release_a_rx.recv().unwrap();
            // `_lock` drops here, releasing the flock.
        });

        a_holds_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("thread A should acquire the lock promptly");

        let dir_b = dir.clone();
        let thread_b = std::thread::spawn(move || {
            let _lock =
                ProvisionLock::acquire(&dir_b).expect("thread B eventually acquires the lock");
            b_acquired_tx.send(()).unwrap();
        });

        let b_raced_in_early = b_acquired_rx
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_ok();
        assert!(
            !b_raced_in_early,
            "thread B acquired the install lock while thread A still held it"
        );

        release_a_tx.send(()).unwrap();
        b_acquired_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("thread B should acquire the lock once thread A releases it");

        thread_a.join().unwrap();
        thread_b.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dead_lock_owner_is_stale() {
        let dir = temp_dir("stale-lock");
        let dead_pid = u32::MAX;
        std::fs::write(dir.join(INSTALL_LOCK), format!("{dead_pid}\n")).unwrap();
        assert!(recorded_lock_is_stale(&dir));

        std::fs::write(dir.join(INSTALL_LOCK), format!("{}\n", std::process::id())).unwrap();
        assert!(!recorded_lock_is_stale(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn streaming_capture_drains_both_pipes_without_deadlocking() {
        // `run_capture(..., stream: true)` — the `InstallPolicy::stream_output:
        // true` path the CLI uses — pipes stdout and stderr and must drain
        // them CONCURRENTLY. A regression to reading one pipe to EOF before
        // starting the other would deadlock here: `sh` below writes enough
        // to each stream to exceed a 64 KiB Linux pipe buffer, so the child
        // blocks writing to whichever stream isn't being read once its
        // buffer fills, and a sequential implementation would then wait
        // forever for the *other* stream to reach EOF (which it never will,
        // since the child is stuck). `tokio::time::timeout` turns that
        // failure mode into a normal assertion failure instead of a hung
        // test / CI job.
        //
        // Deliberately a `sh` loop, not `npm`/`npx` — no network, no real
        // install, per the brief's instruction to prove the streaming
        // mechanism itself without spawning a real installer.
        let dir = temp_dir("stream-no-deadlock");
        let script = "i=0; while [ $i -lt 4000 ]; do \
             echo \"o-$i-0123456789012345678901234567890123456789\"; \
             echo \"e-$i-0123456789012345678901234567890123456789\" 1>&2; \
             i=$((i+1)); \
             done";

        let out = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            run_capture("sh", &["-c", script], &dir, &[], true),
        )
        .await
        .expect("run_capture must not deadlock draining stdout+stderr concurrently")
        .expect("`sh` must spawn");

        assert!(out.ok, "script should exit 0:\n{}", out.text);
        // Both streams' first and last lines must have made it into the
        // accumulated text — proof neither pipe was silently truncated or
        // starved by the other.
        for needle in ["o-0-", "o-3999-", "e-0-", "e-3999-"] {
            assert!(
                out.text.contains(needle),
                "missing {needle:?} in accumulated text (len {})",
                out.text.len()
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn streaming_capture_survives_an_invalid_utf8_line() {
        // Regression: `AsyncBufReadExt::lines()` yields `Err` — not
        // `None` — the moment a line isn't valid UTF-8. The earlier
        // `while let Ok(Some(line)) = lines.next_line().await` form of
        // `tee_lines` treated that `Err` exactly like EOF, so one stray
        // non-UTF-8 byte silently stopped draining *this* pipe. With one
        // side of the `tokio::join!` no longer being read, the child
        // eventually blocks writing to it once its OS buffer fills, and
        // `child.wait()` hangs — defeating the very concurrent-drain the
        // deadlock test above exists to prove. `tee_lines` now reads
        // bytes (`read_until(b'\n', ..)`) and lossily decodes each line,
        // so invalid UTF-8 is never an `Err` to mistake for EOF.
        //
        // `printf` (not npm/npx — no network) writes a normal line, then
        // one line that's a single invalid UTF-8 byte (`\377` = 0xFF,
        // never valid as a UTF-8 lead or continuation byte), then one
        // more normal line after it.
        let dir = temp_dir("stream-invalid-utf8");
        let script = r"printf 'before\n\377\nafter\n'";

        let out = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            run_capture("sh", &["-c", script], &dir, &[], true),
        )
        .await
        .expect("run_capture must not hang on an invalid-UTF-8 line")
        .expect("`sh` must spawn");

        assert!(out.ok, "script should exit 0:\n{:?}", out.text);
        assert!(
            out.text.contains("before"),
            "line before the bad byte is missing: {:?}",
            out.text
        );
        assert!(
            out.text.contains("after"),
            "line after the bad byte is missing — draining stopped at the bad line: {:?}",
            out.text
        );
        // The malformed line itself must survive too, lossily decoded
        // (U+FFFD standing in for the bad byte) rather than vanishing —
        // `install_browser`'s distro-refusal classifier inspects this
        // exact accumulated text, so a dropped line could hide a real
        // refusal message that happened to contain a stray byte.
        assert!(
            out.text.contains('\u{FFFD}'),
            "malformed line should survive as U+FFFD, not disappear: {:?}",
            out.text
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
