//! `duhem run --reporter` stdout formatters.
//!
//! Two surfaces:
//!
//! - **Built-in reporters** (spec on issue #23): `default` / `quiet` /
//!   `json`. Their output is the CLI's externally-frozen baseline.
//! - **Plugin reporters** (spec on issue #34): author-supplied
//!   subprocesses. The CLI writes one line of [`RunSummary`] JSON to
//!   the plugin's stdin, captures its stdout, and propagates exit
//!   code (non-zero → reporter error, distinct from the verification
//!   verdict). Discovery is via `.duhem.toml` (repo) and
//!   `~/.duhem/config.toml` (user). Resolution order from `main.rs`:
//!   built-in match → repo config → user config → error. Built-ins
//!   are not shadowable.
//!
//! Reporters format the post-run summary only. `trace.jsonl` is
//! identical regardless of the chosen reporter.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use duhem_runtime::RunOutcome;
use duhem_summary::{CriterionSummary, RunSummary};

/// Selectable reporter. Built-ins are tagged variants; plugins carry
/// the full argv they were discovered with so the dispatch is uniform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reporter {
    Default,
    Quiet,
    Json,
    /// Author-supplied subprocess reporter. `argv[0]` is the program,
    /// `argv[1..]` are its arguments. The `name` field is the
    /// `--reporter <name>` value the user typed; it's not passed to
    /// the subprocess but appears in error messages.
    Plugin {
        name: String,
        argv: Vec<String>,
    },
}

/// Errors returned by [`render`] that need to surface to the CLI
/// dispatcher. Plain I/O errors from stdout flushing stay as
/// `std::io::Error` and are handled by the caller.
#[derive(Debug)]
pub enum RenderError {
    /// Plain stdout write failure (built-in reporters).
    Io(std::io::Error),
    /// Plugin subprocess failed to spawn (program not on PATH, etc).
    PluginSpawn {
        name: String,
        source: std::io::Error,
    },
    /// Plugin exited with a non-zero status. The CLI treats this as a
    /// reporter error distinct from the verification verdict —
    /// `Outcome::Fail` from the run is still authoritative.
    PluginExit {
        name: String,
        code: Option<i32>,
        stderr: String,
    },
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::Io(e) => write!(f, "{e}"),
            RenderError::PluginSpawn { name, source } => {
                write!(f, "reporter `{name}`: failed to spawn: {source}")
            }
            RenderError::PluginExit { name, code, stderr } => {
                let c = code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                if stderr.is_empty() {
                    write!(f, "reporter `{name}`: exited with status {c}")
                } else {
                    write!(
                        f,
                        "reporter `{name}`: exited with status {c}: {}",
                        stderr.trim_end()
                    )
                }
            }
        }
    }
}

impl std::error::Error for RenderError {}

impl From<std::io::Error> for RenderError {
    fn from(e: std::io::Error) -> Self {
        RenderError::Io(e)
    }
}

/// Render the post-run summary for `outcome` to `out`. Reporter
/// selection is a stdout-only concern; the writer is parametric so
/// tests can capture output without going through the real stdout.
pub fn render(
    reporter: &Reporter,
    out: &mut dyn Write,
    outcome: &RunOutcome,
) -> Result<(), RenderError> {
    match reporter {
        Reporter::Default => {
            writeln!(out, "{}", outcome.verdict.state)?;
            Ok(())
        }
        Reporter::Quiet => Ok(()),
        Reporter::Json => {
            let summary = build_summary(outcome);
            // One JSON object per run, newline-terminated. Authors
            // who want bulk-parsing get JSON-lines-friendly output.
            serde_json::to_writer(&mut *out, &summary)
                .map_err(|e| RenderError::Io(std::io::Error::other(e)))?;
            writeln!(out)?;
            Ok(())
        }
        Reporter::Plugin { name, argv } => render_plugin(name, argv, out, outcome),
    }
}

fn build_summary(o: &RunOutcome) -> RunSummary {
    RunSummary::new(
        o.run_id.clone(),
        o.verdict.state,
        o.verdict
            .criteria
            .iter()
            .map(|c| CriterionSummary {
                id: c.criterion_id.clone(),
                verdict: c.state,
            })
            .collect(),
        o.run_dir.clone(),
    )
}

/// Spawn a plugin subprocess, write the `RunSummary` JSON line to its
/// stdin, copy its stdout to `out`, and propagate any non-zero exit
/// code as a `RenderError`. Stderr is captured and inlined into the
/// error message so author plugins can fail loudly.
fn render_plugin(
    name: &str,
    argv: &[String],
    out: &mut dyn Write,
    outcome: &RunOutcome,
) -> Result<(), RenderError> {
    // argv is non-empty per `PluginRegistry::load`'s validation, so
    // [0] is safe.
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| RenderError::PluginSpawn {
        name: name.to_string(),
        source: e,
    })?;

    let summary = build_summary(outcome);
    let mut stdin = child
        .stdin
        .take()
        .expect("stdin pipe is configured above; take() must succeed");
    // Write inside a scoped block so stdin closes before we wait on
    // the child. Plugins that block on EOF need this; without it
    // child.wait() would deadlock waiting for them to exit.
    {
        let line =
            serde_json::to_vec(&summary).map_err(|e| RenderError::Io(std::io::Error::other(e)))?;
        stdin.write_all(&line)?;
        stdin.write_all(b"\n")?;
        // Drop closes the pipe.
    }
    drop(stdin);

    let mut stdout = child
        .stdout
        .take()
        .expect("stdout pipe is configured above");
    let mut buf = Vec::new();
    stdout.read_to_end(&mut buf)?;
    out.write_all(&buf)?;

    let mut stderr = child
        .stderr
        .take()
        .expect("stderr pipe is configured above");
    let mut err_buf = Vec::new();
    let _ = stderr.read_to_end(&mut err_buf);
    let stderr_text = String::from_utf8_lossy(&err_buf).into_owned();

    let status = child.wait().map_err(RenderError::Io)?;
    if !status.success() {
        return Err(RenderError::PluginExit {
            name: name.to_string(),
            code: status.code(),
            stderr: stderr_text,
        });
    }
    Ok(())
}

/// Convert the `Reporter::Json` summary into its serialized JSON
/// form. Exposed for tests that want to assert on the contract shape
/// without going through stdout.
#[cfg(test)]
pub(crate) fn json_line_for(outcome: &RunOutcome) -> String {
    let summary = build_summary(outcome);
    serde_json::to_string(&summary).unwrap()
}

/// Helper used by `main::resolve_reporter` to look up a name and
/// produce the matching `Reporter`. Returns `Ok(Reporter::Plugin)`
/// only if `name` is in `registry` AND not one of the built-in
/// reserved names. Built-ins are never shadowable.
pub fn resolve_by_name(
    name: &str,
    registry: &crate::reporter_config::PluginRegistry,
) -> Result<Reporter, String> {
    match name {
        "default" => Ok(Reporter::Default),
        "quiet" => Ok(Reporter::Quiet),
        "json" => Ok(Reporter::Json),
        other => match registry.get(other) {
            Some(entry) => Ok(Reporter::Plugin {
                name: other.to_string(),
                argv: entry.command.clone(),
            }),
            None => Err(format!("unknown reporter: {other}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reporter_config::{PluginEntry, PluginRegistry};
    use duhem_judge::{
        CheckVerdict, CriterionVerdict, InconclusiveCause, RunVerdict, VerdictState,
    };
    use std::path::PathBuf;

    fn outcome(state: VerdictState) -> RunOutcome {
        RunOutcome {
            verdict: RunVerdict {
                state,
                criteria: vec![CriterionVerdict {
                    criterion_id: "AC-1".into(),
                    state,
                    checks: vec![CheckVerdict {
                        check_id: "AC-1.1".into(),
                        state,
                    }],
                }],
            },
            run_id: "01J000000000000000000RUN".into(),
            run_dir: PathBuf::from(".duhem/runs/01J000000000000000000RUN"),
        }
    }

    fn capture(reporter: &Reporter, o: &RunOutcome) -> String {
        let mut buf = Vec::new();
        render(reporter, &mut buf, o).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn default_reporter_writes_single_verdict_line() {
        let s = capture(&Reporter::Default, &outcome(VerdictState::Pass));
        assert_eq!(s, "pass\n");
    }

    #[test]
    fn default_reporter_emits_inconclusive_state() {
        let s = capture(
            &Reporter::Default,
            &outcome(VerdictState::Inconclusive(InconclusiveCause::Timeout)),
        );
        assert_eq!(s, "inconclusive:timeout\n");
    }

    #[test]
    fn quiet_reporter_writes_nothing() {
        let s = capture(&Reporter::Quiet, &outcome(VerdictState::Fail));
        assert_eq!(s, "");
    }

    #[test]
    fn json_reporter_is_single_line_valid_json() {
        let s = capture(&Reporter::Json, &outcome(VerdictState::Pass));
        let trimmed = s.trim_end_matches('\n');
        assert!(!trimmed.contains('\n'), "single line: {s:?}");
        let v: serde_json::Value = serde_json::from_str(trimmed).expect("valid JSON");
        assert_eq!(v["run_id"], "01J000000000000000000RUN");
        assert_eq!(v["verdict"], "pass");
        assert_eq!(v["criteria"][0]["id"], "AC-1");
        assert_eq!(v["criteria"][0]["verdict"], "pass");
        assert_eq!(v["evidence_dir"], ".duhem/runs/01J000000000000000000RUN");
        // Spec on #34: the contract surfaces schema_version on the wire.
        assert_eq!(v["schema_version"], "1");
    }

    #[test]
    fn json_reporter_emits_inconclusive_wire_form() {
        let s = capture(
            &Reporter::Json,
            &outcome(VerdictState::Inconclusive(
                InconclusiveCause::MissingObservation,
            )),
        );
        let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(v["verdict"], "inconclusive:missing_observation");
    }

    #[test]
    fn resolve_built_in_wins_even_when_shadowed_in_config() {
        // Spec on #34: built-ins are not shadowable. Even if a config
        // file declares `json`, `--reporter json` must reach the
        // built-in (`Reporter::Json`), not the plugin.
        let registry = PluginRegistry::from_entries([(
            "json".to_string(),
            PluginEntry {
                command: vec!["fake-json".to_string()],
            },
        )])
        .unwrap();
        let r = resolve_by_name("json", &registry).unwrap();
        assert_eq!(r, Reporter::Json);
    }

    #[test]
    fn resolve_plugin_returns_argv_from_registry() {
        let registry = PluginRegistry::from_entries([(
            "pretty".to_string(),
            PluginEntry {
                command: vec!["duhem-reporter-pretty".to_string()],
            },
        )])
        .unwrap();
        let r = resolve_by_name("pretty", &registry).unwrap();
        match r {
            Reporter::Plugin { name, argv } => {
                assert_eq!(name, "pretty");
                assert_eq!(argv, vec!["duhem-reporter-pretty".to_string()]);
            }
            other => panic!("expected Plugin, got {other:?}"),
        }
    }

    #[test]
    fn resolve_unknown_name_errors() {
        let registry = PluginRegistry::default();
        let err = resolve_by_name("nope", &registry).unwrap_err();
        assert!(
            err.contains("unknown reporter") && err.contains("nope"),
            "got: {err}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn plugin_subprocess_receives_run_summary_on_stdin_and_round_trips_stdout() {
        // Spec on #34 Test § "fake `[bash, -c, cat]` reporter receives
        // RunSummary on stdin and round-trips byte-identical output".
        let plugin = Reporter::Plugin {
            name: "echo".to_string(),
            argv: vec!["/bin/cat".to_string()],
        };
        let o = outcome(VerdictState::Pass);
        let captured = capture(&plugin, &o);
        let expected = json_line_for(&o);
        // `cat` doesn't add a trailing newline beyond what was sent;
        // the launcher writes the json line plus `\n` to stdin, so the
        // expected round-trip is the json + newline.
        assert_eq!(captured, format!("{expected}\n"));
    }

    #[test]
    #[cfg(unix)]
    fn plugin_nonzero_exit_surfaces_as_render_error() {
        // Spec on #34 Test § "unknown name yields exit 2 with `unknown
        // reporter:` on stderr." We test the adjacent shape: a plugin
        // that returns non-zero status must surface as a `RenderError`,
        // not silently succeed.
        let plugin = Reporter::Plugin {
            name: "boom".to_string(),
            argv: vec!["/bin/false".to_string()],
        };
        let o = outcome(VerdictState::Pass);
        let mut buf = Vec::new();
        let err = render(&plugin, &mut buf, &o).unwrap_err();
        match err {
            RenderError::PluginExit { name, code, .. } => {
                assert_eq!(name, "boom");
                assert_eq!(code, Some(1));
            }
            other => panic!("expected PluginExit, got {other}"),
        }
    }

    #[test]
    fn plugin_missing_program_surfaces_as_spawn_error() {
        let plugin = Reporter::Plugin {
            name: "nope".to_string(),
            argv: vec!["/no/such/binary".to_string()],
        };
        let o = outcome(VerdictState::Pass);
        let mut buf = Vec::new();
        let err = render(&plugin, &mut buf, &o).unwrap_err();
        match err {
            RenderError::PluginSpawn { name, .. } => assert_eq!(name, "nope"),
            other => panic!("expected PluginSpawn, got {other}"),
        }
    }
}
