//! Environment-provisioning lifecycle — operator-supplied `up:` /
//! `down:` scripts + a readiness probe.
//!
//! Per the spec on issue #50, `environment:` brings Stage 3
//! ("Provision Environment") from `docs/duhem-spec.md` §9 under the
//! runtime's control. `up:` runs once before `setup:`; `ready:` is
//! polled before `setup:` starts; `down:` runs once after the last
//! criterion (regardless of verdict). Failure policy is
//! three-state-faithful: `up:` non-zero or `ready:` timeout →
//! `Inconclusive` — we cannot observe the workload in the verified
//! state, so we don't know, not "fail" (same reasoning as the
//! setup-failure policy on issue #20).
//!
//! Child-process discipline: the operator's script is treated as
//! untrusted-but-deterministic. Env is sanitized to a small whitelist
//! (`PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_*`, `DUHEM_*`); cwd is the
//! Verification Definition's directory. Stdout/stderr are captured
//! and recorded as content-addressed blobs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use duhem_evidence::{EventPayload, EvidenceWriter, new_run_id};
use duhem_judge::InconclusiveCause;
use duhem_schema::{Environment, HttpReadyProbe, ReadyProbe};
use tokio::process::Command;
use tracing::debug;

use crate::engine::context::RunState;
use crate::engine::runner::EngineError;

/// Probe-kind wire token emitted by `EnvReady` events. v1 only knows
/// `http`; new probe kinds widen the catalog without renaming this
/// field on the event.
const PROBE_KIND_HTTP: &str = "http";

/// Why `environment.up:` / `ready:` aborted the run. Same shape as
/// `setup::AbortReason` — the engine maps the trigger to a
/// `RunVerdict::Inconclusive(cause)` and records evidence accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnvAbortReason {
    /// `ready:` exhausted its timeout. Maps to
    /// `Inconclusive(Timeout)` on the run verdict.
    Timeout,
    /// `up:` exited non-zero, was unrunnable (missing binary, IO
    /// error), or `ready:` failed with a non-timeout error. Maps to
    /// `Inconclusive(EnvironmentError)`.
    Environment,
}

impl EnvAbortReason {
    pub fn cause(self) -> InconclusiveCause {
        match self {
            EnvAbortReason::Timeout => InconclusiveCause::Timeout,
            EnvAbortReason::Environment => InconclusiveCause::EnvironmentError,
        }
    }
}

/// Outcome of `bring_environment_up`. `aborted: None` means up
/// succeeded and readiness was observed (or no probe was declared);
/// the engine proceeds to setup/criteria. `should_tear_down: true`
/// means teardown should still run — either `up:` exited 0
/// (Duhem provisioned, Duhem cleans up), or `up:` was skipped via
/// `--no-env-up` (the operator opted into Duhem-managed teardown
/// against a pre-booted SUT, and can pass `--keep-env` if they
/// don't want that). A non-zero `up:` exit pins this to `false`:
/// nothing came up, so there is nothing for `down:` to undo.
pub(crate) struct EnvUpResult {
    pub aborted: Option<EnvAbortReason>,
    pub should_tear_down: bool,
}

/// Bring the environment up: fork `up:`, await exit, poll the
/// readiness probe. Skipped entirely when `skip_env_up` is true (the
/// `--no-env-up` escape hatch), in which case the runtime trusts the
/// operator brought the SUT up by hand.
pub(crate) async fn bring_environment_up(
    writer: &mut EvidenceWriter,
    env: &Environment,
    vd_dir: Option<&Path>,
    run: &RunState,
    skip_env_up: bool,
) -> Result<EnvUpResult, EngineError> {
    if skip_env_up {
        debug!("--no-env-up: skipping environment.up + readiness probe");
        // The operator opted into Duhem-managed teardown against a
        // pre-booted SUT; if they want both halves skipped they pass
        // `--keep-env` as well.
        return Ok(EnvUpResult {
            aborted: None,
            should_tear_down: true,
        });
    }

    let up_script = resolve_script_path(&env.up, vd_dir);
    let command_str = up_script.display().to_string();
    writer
        .append(EventPayload::EnvUpStarted {
            command: command_str.clone(),
        })
        .await?;

    let (exit_code, duration, stdout, stderr) = run_script(&up_script, vd_dir).await;
    let stdout_blob = write_blob_if_nonempty(writer, &stdout).await?;
    let stderr_blob = write_blob_if_nonempty(writer, &stderr).await?;
    writer
        .append(EventPayload::EnvUpFinished {
            exit_code,
            duration_ms: duration_ms_u64(duration),
            stdout_blob_sha256: stdout_blob,
            stderr_blob_sha256: stderr_blob,
        })
        .await?;

    if exit_code != 0 {
        // `up:` failed: nothing was provisioned, so teardown must
        // not run.
        return Ok(EnvUpResult {
            aborted: Some(EnvAbortReason::Environment),
            should_tear_down: false,
        });
    }

    if let Some(probe) = &env.ready {
        let (ok, elapsed, kind) = match probe {
            ReadyProbe::Http(p) => {
                let (ok, elapsed) = poll_http_ready(p, run).await;
                (ok, elapsed, PROBE_KIND_HTTP)
            }
        };
        writer
            .append(EventPayload::EnvReady {
                probe_kind: kind.to_string(),
                ok,
                elapsed_ms: duration_ms_u64(elapsed),
            })
            .await?;
        if !ok {
            // `up:` succeeded but the SUT never became ready. Teardown
            // still runs so the half-booted SUT cleans up after
            // itself.
            return Ok(EnvUpResult {
                aborted: Some(EnvAbortReason::Timeout),
                should_tear_down: true,
            });
        }
    }

    Ok(EnvUpResult {
        aborted: None,
        should_tear_down: true,
    })
}

/// Tear the environment down. Best-effort: teardown failures are
/// recorded as evidence but never change the run verdict. Skipped
/// when `keep_env` is true (the `--keep-env` debug flag), when
/// `down:` is not declared, or when caller signals no teardown
/// (e.g. a failed `up:` provisioned nothing).
pub(crate) async fn tear_environment_down(
    writer: &mut EvidenceWriter,
    env: &Environment,
    vd_dir: Option<&Path>,
    keep_env: bool,
    should_tear_down: bool,
) -> Result<(), EngineError> {
    if keep_env {
        debug!("--keep-env: skipping environment.down");
        return Ok(());
    }
    if !should_tear_down {
        return Ok(());
    }
    let Some(down) = env.down.as_ref() else {
        return Ok(());
    };
    let down_script = resolve_script_path(down, vd_dir);
    writer
        .append(EventPayload::EnvDownStarted {
            command: down_script.display().to_string(),
        })
        .await?;
    let (exit_code, duration, stdout, stderr) = run_script(&down_script, vd_dir).await;
    let stdout_blob = write_blob_if_nonempty(writer, &stdout).await?;
    let stderr_blob = write_blob_if_nonempty(writer, &stderr).await?;
    writer
        .append(EventPayload::EnvDownFinished {
            exit_code,
            duration_ms: duration_ms_u64(duration),
            stdout_blob_sha256: stdout_blob,
            stderr_blob_sha256: stderr_blob,
        })
        .await?;
    Ok(())
}

/// A manifest's shared environment, provisioned **once** for the whole
/// suite (spec #131). The CLI provisions before any leaf runs and tears
/// down after the last leaf, instead of each leaf standing up its own
/// stack. Suite-level `up:` / `ready:` / `down:` evidence is recorded
/// as its own run in the store, distinct from each leaf's run.
pub struct SuiteEnvironment {
    env: Environment,
    dir: Option<PathBuf>,
    writer: EvidenceWriter,
    run: RunState,
    should_tear_down: bool,
    aborted: Option<EnvAbortReason>,
}

impl SuiteEnvironment {
    /// Provision the shared environment. `manifest_dir` anchors relative
    /// `up:` / `down:` script paths; `store` is where the suite-level
    /// evidence run lands. `skip_env_up` is the manifest-level
    /// `--no-env-up`. `progress` (#305) mirrors the suite run's events
    /// to a live subscriber — the same post-commit tee as
    /// `Engine::with_progress`, covering `tear_down` too since the
    /// writer carries it; `None` is byte-for-byte the prior behavior.
    pub async fn provision(
        env: &Environment,
        manifest_dir: Option<&Path>,
        store: std::sync::Arc<dyn duhem_evidence::Store>,
        skip_env_up: bool,
        progress: Option<tokio::sync::mpsc::UnboundedSender<duhem_evidence::Event>>,
    ) -> Result<Self, EngineError> {
        let run = RunState::new(std::collections::BTreeMap::new());
        // The suite's shared-environment evidence is its own run row,
        // identified by the `<suite>` verification marker.
        let mut writer = EvidenceWriter::begin(
            store,
            new_run_id(),
            "<suite>",
            std::collections::BTreeMap::new(),
        )
        .await?;
        if let Some(tx) = progress {
            writer = writer.with_tee(tx);
        }
        let up = bring_environment_up(&mut writer, env, manifest_dir, &run, skip_env_up).await?;
        Ok(Self {
            env: env.clone(),
            dir: manifest_dir.map(Path::to_path_buf),
            writer,
            run,
            should_tear_down: up.should_tear_down,
            aborted: up.aborted,
        })
    }

    /// Why provisioning aborted (failed `up:` → `EnvironmentError`,
    /// readiness `Timeout`), if it did. `None` means the suite is up.
    pub fn aborted_cause(&self) -> Option<InconclusiveCause> {
        self.aborted.map(EnvAbortReason::cause)
    }

    /// Tear the shared environment down (best-effort) and close the
    /// suite trace. `keep_env` is the manifest-level `--keep-env`.
    pub async fn tear_down(mut self, keep_env: bool) -> Result<(), EngineError> {
        tear_environment_down(
            &mut self.writer,
            &self.env,
            self.dir.as_deref(),
            keep_env,
            self.should_tear_down,
        )
        .await?;
        self.writer.finish().await?;
        Ok(())
    }

    // Keep `run` reachable for a future manifest-input-resolving probe;
    // today the readiness URL is resolved against an empty input set.
    #[allow(dead_code)]
    fn run(&self) -> &RunState {
        &self.run
    }
}

/// Resolve a script path. Relative paths anchor at the VD directory
/// when known; falling back to cwd preserves programmatic-caller
/// behavior (engines built without `with_definition_path` keep
/// working).
fn resolve_script_path(path: &Path, vd_dir: Option<&Path>) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match vd_dir {
        Some(dir) => dir.join(path),
        None => path.to_path_buf(),
    }
}

/// Fork the script with a sanitized environment, collect stdout /
/// stderr, and return `(exit_code, wall_time, stdout, stderr)`.
/// `exit_code` is `-1` on signal exit and `-2` on spawn failure so
/// the caller can distinguish "ran and failed" from "could not run".
///
/// The child's cwd is the Verification Definition's directory when
/// known (so author-relative paths inside the script — `./scripts/`,
/// fixture lookups, etc. — resolve from the same anchor as
/// `environment.up:` itself). When the VD path is unknown, the
/// runtime inherits cwd from the parent process; we deliberately do
/// NOT set cwd to `script.parent()`, which would (a) contradict the
/// "cwd = VD directory" contract and (b) silently break the
/// relative-path fallback (a script invoked as `./scripts/up.sh`
/// would re-resolve as `./scripts/./scripts/up.sh` after the cwd
/// change). Scripts that need their own directory can compute it
/// from `$0` / `argv[0]`.
async fn run_script(script: &Path, vd_dir: Option<&Path>) -> (i32, Duration, Vec<u8>, Vec<u8>) {
    let started = Instant::now();
    // Resolve the program to an absolute path before spawning. The child
    // runs with `current_dir(vd_dir)` below, and on Unix a *relative*
    // program path is re-resolved against the child's new cwd — so a
    // path like `<vd_dir>/./scripts/up.sh` would double to
    // `<vd_dir>/<vd_dir>/scripts/up.sh` and fail with ENOENT. Anchoring
    // the program absolutely (against the process cwd) keeps it found
    // while the script still *runs* with cwd = the VD directory.
    let program = if script.is_absolute() {
        script.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(script))
            .unwrap_or_else(|_| script.to_path_buf())
    };
    let mut cmd = Command::new(&program);
    cmd.env_clear();
    for (k, v) in sanitized_env_vars(std::env::vars()) {
        cmd.env(k, v);
    }
    if let Some(dir) = vd_dir
        && !dir.as_os_str().is_empty()
    {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => {
            let msg = format!("spawn failed: {e}\n");
            return (-2, started.elapsed(), Vec::new(), msg.into_bytes());
        }
    };
    let exit_code = output.status.code().unwrap_or(-1);
    (exit_code, started.elapsed(), output.stdout, output.stderr)
}

/// Whitelist for the sanitized child environment. Per the issue
/// alignment: `PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_*`, and
/// `DUHEM_*`. Everything else (including attacker-shaped vars like
/// `LD_PRELOAD`) is dropped before the script is forked.
pub(crate) fn sanitized_env_vars<I>(iter: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    let exact: HashSet<&str> = ["PATH", "HOME", "TMPDIR", "LANG"].into_iter().collect();
    iter.into_iter()
        .filter(|(k, _)| {
            exact.contains(k.as_str()) || k.starts_with("LC_") || k.starts_with("DUHEM_")
        })
        .collect()
}

/// Poll the HTTP probe until either the expected status is observed
/// or the configured timeout elapses. Polling cadence is fixed at
/// 500 ms — a coarser-grained value than necessary for "did the
/// server come up" because finer granularity buys nothing for boot
/// scripts that take seconds to start (per the issue's worked
/// example `timeout: 60s`).
async fn poll_http_ready(probe: &HttpReadyProbe, run: &RunState) -> (bool, Duration) {
    let started = Instant::now();
    let total: Duration = probe.timeout.into();
    let url = resolve_url(&probe.url, run);

    // Per-request timeout below total so a hanging GET cannot starve
    // the budget; cap at 2s so the readiness loop stays responsive.
    let per_req = std::cmp::min(total, Duration::from_secs(2));
    let client = match reqwest::Client::builder().timeout(per_req).build() {
        Ok(c) => c,
        Err(_) => return (false, started.elapsed()),
    };

    loop {
        if let Ok(resp) = client.get(&url).send().await
            && resp.status().as_u16() == probe.expect_status
        {
            return (true, started.elapsed());
        }
        if started.elapsed() >= total {
            return (false, started.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Resolve a single whole-string `$inputs.<name>` reference in the
/// readiness URL. Anything more elaborate (path-joins, partial
/// substitution) is out of scope at v1 — same conservative substitution
/// the `Step.with` template path uses (`engine/template.rs`).
fn resolve_url(raw: &str, run: &RunState) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with('$') {
        return raw.to_string();
    }
    let Some(rest) = trimmed.strip_prefix("$inputs.") else {
        return raw.to_string();
    };
    let name = rest.trim();
    if name.is_empty() || name.contains('.') || name.contains(' ') {
        return raw.to_string();
    }
    match run.inputs.get(name) {
        Some(crate::eval::Value::Str(s)) => s.clone(),
        _ => raw.to_string(),
    }
}

async fn write_blob_if_nonempty(
    writer: &mut EvidenceWriter,
    bytes: &[u8],
) -> Result<Option<String>, EngineError> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let sha = writer.write_blob(bytes).await?;
    Ok(Some(sha.0))
}

fn duration_ms_u64(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitization_keeps_whitelisted_and_drops_everything_else() {
        let raw = vec![
            ("PATH".into(), "/usr/bin".into()),
            ("HOME".into(), "/home/x".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("LC_ALL".into(), "C".into()),
            ("DUHEM_RUN_ID".into(), "01J".into()),
            // The attacker-shaped vars the spec explicitly names.
            ("LD_PRELOAD".into(), "/evil.so".into()),
            ("RANDOM_OTHER".into(), "x".into()),
        ];
        let out: Vec<_> = sanitized_env_vars(raw)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert!(out.contains(&"PATH".to_string()));
        assert!(out.contains(&"HOME".to_string()));
        assert!(out.contains(&"LANG".to_string()));
        assert!(out.contains(&"LC_ALL".to_string()));
        assert!(out.contains(&"DUHEM_RUN_ID".to_string()));
        assert!(!out.contains(&"LD_PRELOAD".to_string()));
        assert!(!out.contains(&"RANDOM_OTHER".to_string()));
    }

    #[test]
    fn relative_script_path_anchors_at_vd_directory() {
        let vd = PathBuf::from("/tmp/vd");
        let resolved = resolve_script_path(Path::new("./scripts/up.sh"), Some(&vd));
        assert_eq!(resolved, PathBuf::from("/tmp/vd/./scripts/up.sh"));
    }

    #[test]
    fn absolute_script_path_passes_through() {
        let resolved = resolve_script_path(Path::new("/opt/up.sh"), Some(Path::new("/tmp/vd")));
        assert_eq!(resolved, PathBuf::from("/opt/up.sh"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_script_spawns_with_vd_join_path_and_anchors_cwd() {
        // Regression: `run_script` sets `current_dir(vd_dir)`, and on Unix
        // a *relative* program path is re-resolved against the child's new
        // cwd — so a `<vd_dir>/./scripts/up.sh` program would double to
        // `<vd_dir>/<vd_dir>/...` and spawn-fail with ENOENT (exit -2).
        // The fix anchors the program absolutely while the script still
        // runs with cwd = the VD directory. We assert both: it ran (exit
        // 0, not -2) and `$PWD` inside the script was the VD directory.
        use std::os::unix::fs::PermissionsExt;

        let base = std::env::temp_dir().join(format!("duhem-runscript-{}", std::process::id()));
        let scripts = base.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let script = scripts.join("up.sh");
        // Exit 0 only if the script's cwd is the VD directory.
        std::fs::write(
            &script,
            format!("#!/bin/sh\n[ \"$PWD\" = \"{}\" ]\n", base.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        // The `vd_dir.join("./scripts/up.sh")` shape the engine produces.
        let resolved = resolve_script_path(Path::new("./scripts/up.sh"), Some(&base));
        let (exit_code, _dur, _out, err) = run_script(&resolved, Some(&base)).await;

        std::fs::remove_dir_all(&base).ok();
        assert_eq!(
            exit_code,
            0,
            "spawn/cwd failed; stderr: {}",
            String::from_utf8_lossy(&err)
        );
    }

    #[test]
    fn url_substitutes_whole_string_input_reference() {
        let mut inputs = std::collections::BTreeMap::new();
        inputs.insert(
            "base_url".to_string(),
            crate::eval::Value::Str("http://localhost:3000/healthz".to_string()),
        );
        let run = RunState::new(inputs);
        let resolved = resolve_url("$inputs.base_url", &run);
        assert_eq!(resolved, "http://localhost:3000/healthz");
    }

    #[test]
    fn url_leaves_plain_strings_alone() {
        let run = RunState::new(std::collections::BTreeMap::new());
        let resolved = resolve_url("http://localhost:3000/health", &run);
        assert_eq!(resolved, "http://localhost:3000/health");
    }

    #[tokio::test]
    async fn suite_environment_provisions_and_tears_down_once() {
        use std::io::Write;
        let base = std::env::temp_dir().join(format!("duhem-suite-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let marker = base.join("marker");

        let write_script = |name: &str, word: &str| {
            let p = base.join(name);
            let mut f = std::fs::File::create(&p).unwrap();
            writeln!(f, "#!/bin/sh\necho {word} >> {}", marker.display()).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        };
        write_script("up.sh", "up");
        write_script("down.sh", "down");

        let env = Environment {
            up: PathBuf::from("up.sh"),
            down: Some(PathBuf::from("down.sh")),
            ready: None,
        };
        let store = duhem_evidence::SqliteStore::open(base.join("duhem.db"))
            .await
            .unwrap();

        let session =
            SuiteEnvironment::provision(&env, Some(&base), std::sync::Arc::new(store), false, None)
                .await
                .expect("provision");
        assert!(session.aborted_cause().is_none(), "suite should be up");
        session.tear_down(false).await.expect("tear down");

        let log = std::fs::read_to_string(&marker).unwrap();
        assert_eq!(log.matches("up").count(), 1, "up ran once: {log:?}");
        assert_eq!(log.matches("down").count(), 1, "down ran once: {log:?}");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// #305: the suite-environment run mirrors its evidence events to
    /// a subscribed progress sink, in order, through teardown — the
    /// seam the CLI's suite-scope live narration folds over.
    #[cfg(unix)]
    #[tokio::test]
    async fn suite_tee_mirrors_env_lifecycle_in_order() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("duhem-suite-tee-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        for name in ["up.sh", "down.sh"] {
            let p = base.join(name);
            std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = Environment {
            up: PathBuf::from("up.sh"),
            down: Some(PathBuf::from("down.sh")),
            ready: None,
        };
        let store = duhem_evidence::SqliteStore::open(base.join("duhem.db"))
            .await
            .unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let session = SuiteEnvironment::provision(
            &env,
            Some(&base),
            std::sync::Arc::new(store),
            false,
            Some(tx),
        )
        .await
        .expect("provision");
        assert!(session.aborted_cause().is_none(), "suite should be up");
        session.tear_down(false).await.expect("tear down");

        let mut kinds = Vec::new();
        while let Ok(evt) = rx.try_recv() {
            kinds.push(evt.payload.kind().to_string());
        }
        assert_eq!(
            kinds,
            vec![
                "env_up_started",
                "env_up_finished",
                "env_down_started",
                "env_down_finished"
            ]
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
