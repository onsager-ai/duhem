//! `cli/invoke` — run a command in the SUT environment and capture
//! its result for mechanical assertion.
//!
//! Both new dogfood targets expose first-class CLIs (Arbor's
//! `pnpm factory "<description>"`, Crawlab's Go binary), and the
//! catalog had no way to drive a command and judge its result. This
//! action runs the **real** binary — no shimmed shell, no fake exit
//! code — consistent with the Holistic Verification Principle
//! (`docs/duhem-spec.md` §8).
//!
//! `with:` shape:
//!
//! - `command`: either a shell string (`"pnpm factory \"x\""`, run via
//!   `sh -c`) or an argv array (`["pnpm", "factory", "x"]`, exec'd
//!   directly with no shell). Argv avoids shell-quoting hazards and is
//!   preferred; the string form is the ergonomic default for simple
//!   commands.
//! - `cwd` (optional): working directory for the command. A relative
//!   path resolves against the `duhem` process's working directory (the
//!   directory `duhem run` was invoked from), not the Verification
//!   Definition's directory — pass an absolute path, or an input the
//!   operator sets per environment, when that distinction matters.
//! - `env` (optional): extra environment variables, layered on top of
//!   the sanitized inherited environment.
//! - `stdin` (optional): string written to the child's stdin, then
//!   closed (EOF). Omitted → stdin is `/dev/null`, so a command that
//!   reads stdin gets EOF immediately instead of hanging.
//! - `within` (optional): wall-clock budget. Exceeding it kills the
//!   child and returns `Outcome::Timeout`.
//!
//! Outputs (fixed schema):
//!
//! - `exit_code`: process exit code as an integer. A process killed by
//!   a signal (no code) reports `-1`.
//! - `stdout`: captured standard output (UTF-8 lossy).
//! - `stderr`: captured standard error (UTF-8 lossy).
//!
//! Outcome mapping mirrors `api/call`: a completed process is
//! `Outcome::Ok` *regardless of exit code* — the code is data on the
//! result, and `exit_code == 0` is judged by an assertion, not the
//! action. `within:` exceeded → `Outcome::Timeout`. A spawn / I/O
//! failure (binary not found, permission denied, broken pipe) →
//! `ActionError::Process`, which the engine maps to `Outcome::Error`.
//!
//! Environment: the child starts from a cleared environment populated
//! with the same whitelist the provisioning scripts use (`PATH`,
//! `HOME`, `TMPDIR`, `LANG`, `LC_*`, `DUHEM_*`; source of truth is
//! `duhem-runtime`'s `engine::env::sanitized_env_vars`) plus any
//! explicit `env:`. This keeps `cli/invoke` at the same trust level as
//! the operator-authored `up:` / `down:` scripts.

use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::action::{Action, ActionCtx, ActionResult, DEFAULT_WITHIN};
use crate::error::ActionError;
use crate::with::WithinSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct With {
    command: Command,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    stdin: Option<String>,
    #[serde(default)]
    within: Option<WithinSpec>,
}

/// A command is either a shell string (run via `sh -c`) or an argv
/// vector (exec'd directly). Untagged so authors write the natural YAML
/// — a scalar string or a sequence — without a discriminator.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Command {
    Shell(String),
    Argv(Vec<String>),
}

pub struct Invoke;

#[async_trait]
impl Action for Invoke {
    fn uses(&self) -> &'static str {
        "cli/invoke"
    }

    async fn invoke(
        &self,
        _ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let with: With =
            serde_yml::from_value(with.clone()).map_err(|e| ActionError::InvalidWith {
                action: "cli/invoke",
                source: e,
            })?;
        execute(with).await
    }
}

/// Runs the command. Factored out from `Action::invoke` so the process
/// behavior can be unit-tested without an `ActionCtx`.
pub(crate) async fn execute(with: With) -> Result<ActionResult, ActionError> {
    let timeout: Duration = with.within.map(Into::into).unwrap_or(DEFAULT_WITHIN);

    let mut cmd = match &with.command {
        Command::Shell(s) => {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c").arg(s);
            c
        }
        Command::Argv(argv) => {
            let program = argv.first().ok_or_else(|| {
                ActionError::Process("cli/invoke: command argv is empty".to_string())
            })?;
            let mut c = tokio::process::Command::new(program);
            c.args(&argv[1..]);
            c
        }
    };

    // Sanitized inherited environment + explicit `env:`. Mirrors the
    // provisioning-script whitelist (engine::env::sanitized_env_vars).
    cmd.env_clear();
    for (k, v) in sanitized_env_vars(std::env::vars()) {
        cmd.env(k, v);
    }
    for (k, v) in &with.env {
        cmd.env(k, v);
    }
    if let Some(dir) = &with.cwd {
        cmd.current_dir(dir);
    }

    // No stdin data → `/dev/null` so a command reading stdin gets EOF
    // rather than blocking on the inherited terminal. With data, pipe
    // it. `kill_on_drop` guarantees a timed-out child is reaped when the
    // `wait_with_output` future is dropped below.
    cmd.stdin(if with.stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| ActionError::Process(format!("cli/invoke: spawn failed: {e}")))?;

    if let Some(data) = &with.stdin {
        let mut si = child.stdin.take().ok_or_else(|| {
            ActionError::Process("cli/invoke: stdin pipe unavailable".to_string())
        })?;
        si.write_all(data.as_bytes())
            .await
            .map_err(|e| ActionError::Process(format!("cli/invoke: write stdin: {e}")))?;
        // Dropping `si` closes the pipe so the child sees EOF.
        drop(si);
    }

    // `wait_with_output` drains stdout+stderr concurrently while waiting,
    // so a child that fills a pipe buffer can't deadlock. Racing it with
    // `timeout` and `kill_on_drop` makes the timeout path clean.
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(ActionError::Process(format!(
                "cli/invoke: wait failed: {e}"
            )));
        }
        Err(_elapsed) => return Ok(ActionResult::timeout()),
    };

    // No `code()` means signal termination on Unix; surface -1 so the
    // assertion sees a definite (non-zero) integer rather than null.
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    Ok(ActionResult::ok()
        .with_output("exit_code", serde_json::Value::from(exit_code))
        .with_output("stdout", serde_json::Value::String(stdout))
        .with_output("stderr", serde_json::Value::String(stderr)))
}

/// Whitelist for the child environment — the same set the provisioning
/// scripts get (`PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_*`, `DUHEM_*`).
/// Duplicated from `duhem-runtime`'s `engine::env::sanitized_env_vars`
/// to keep this action crate free of a runtime dependency; the runtime
/// copy is the source of truth.
fn sanitized_env_vars<I>(iter: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    iter.into_iter()
        .filter(|(k, _)| {
            matches!(k.as_str(), "PATH" | "HOME" | "TMPDIR" | "LANG")
                || k.starts_with("LC_")
                || k.starts_with("DUHEM_")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Outcome;

    fn yaml(s: &str) -> serde_yml::Value {
        serde_yml::from_str(s).unwrap()
    }

    fn parse_with(s: &str) -> With {
        serde_yml::from_value(yaml(s)).expect("With deserialization")
    }

    #[test]
    fn parses_shell_string_command() {
        let w = parse_with(r#"{ command: "echo hi" }"#);
        assert!(matches!(w.command, Command::Shell(ref s) if s == "echo hi"));
    }

    #[test]
    fn parses_argv_command() {
        let w = parse_with(r#"{ command: ["echo", "hi"] }"#);
        match w.command {
            Command::Argv(v) => assert_eq!(v, vec!["echo", "hi"]),
            other => panic!("expected argv, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_field() {
        let r: Result<With, _> = serde_yml::from_str(r#"{ command: "x", color: red }"#);
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn captures_stdout_and_zero_exit() {
        let r = execute(parse_with(r#"{ command: "printf hello" }"#))
            .await
            .unwrap();
        assert_eq!(r.outcome, Outcome::Ok);
        assert_eq!(r.outputs.get("exit_code").and_then(|v| v.as_i64()), Some(0));
        assert_eq!(
            r.outputs.get("stdout").and_then(|v| v.as_str()),
            Some("hello")
        );
    }

    #[tokio::test]
    async fn nonzero_exit_is_outcome_ok_with_code() {
        // Exit code is data, not a verdict — like a 500 for api/call.
        let r = execute(parse_with(r#"{ command: "exit 3" }"#))
            .await
            .unwrap();
        assert_eq!(r.outcome, Outcome::Ok);
        assert_eq!(r.outputs.get("exit_code").and_then(|v| v.as_i64()), Some(3));
    }

    #[tokio::test]
    async fn captures_stderr() {
        let r = execute(parse_with(r#"{ command: "printf oops 1>&2" }"#))
            .await
            .unwrap();
        assert_eq!(
            r.outputs.get("stderr").and_then(|v| v.as_str()),
            Some("oops")
        );
    }

    #[tokio::test]
    async fn argv_runs_without_shell() {
        let r = execute(parse_with(r#"{ command: ["printf", "argv-form"] }"#))
            .await
            .unwrap();
        assert_eq!(
            r.outputs.get("stdout").and_then(|v| v.as_str()),
            Some("argv-form")
        );
    }

    #[tokio::test]
    async fn stdin_is_piped_to_the_child() {
        let r = execute(parse_with(r#"{ command: "cat", stdin: "from-stdin" }"#))
            .await
            .unwrap();
        assert_eq!(
            r.outputs.get("stdout").and_then(|v| v.as_str()),
            Some("from-stdin")
        );
    }

    #[tokio::test]
    async fn slow_command_past_within_yields_timeout() {
        let r = execute(parse_with(r#"{ command: "sleep 5", within: 100ms }"#))
            .await
            .unwrap();
        assert_eq!(r.outcome, Outcome::Timeout);
    }

    #[tokio::test]
    async fn missing_binary_yields_process_error() {
        let r = execute(parse_with(
            r#"{ command: ["this-binary-does-not-exist-xyz"] }"#,
        ))
        .await;
        match r {
            Err(ActionError::Process(_)) => {}
            other => panic!("expected ActionError::Process, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn explicit_env_reaches_the_child() {
        let r = execute(parse_with(
            r#"{ command: "printf %s \"$DUHEM_TESTVAR\"", env: { DUHEM_TESTVAR: present } }"#,
        ))
        .await
        .unwrap();
        assert_eq!(
            r.outputs.get("stdout").and_then(|v| v.as_str()),
            Some("present")
        );
    }
}
