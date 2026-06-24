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

use std::io::Write;
use std::process::{Command, Stdio};

use duhem_judge::RunSetVerdict;
use duhem_runtime::CheckFailure;
use duhem_runtime::RunOutcome;
use duhem_summary::{
    CheckFailureSummary, CriterionSummary, FailedAssertionSummary, RunSetSummary, RunSummary,
};

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
            write_failures(out, &outcome.failures)?;
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

/// Render a manifest's per-leaf outcomes plus the aggregated
/// run-set verdict (spec on issue #49). Per-built-in behavior:
///
/// - `Default` — one `<name>: <verdict>` line per leaf, then the
///   aggregated verdict on its own line.
/// - `Quiet` — nothing.
/// - `Json` — a single `RunSetSummary` JSON object on one line.
/// - `Plugin` — fan out: one plugin invocation per leaf (each with
///   the leaf's `RunSummary` on stdin), then the aggregated verdict
///   on stdout from the CLI. The set-level summary is not yet shipped
///   to plugins — issue #49 explicitly calls out `run_set_finished`
///   as an optional, no-op-by-default extension.
pub fn render_set(
    reporter: &Reporter,
    out: &mut dyn Write,
    leaves: &[(String, RunOutcome)],
    set_verdict: &RunSetVerdict,
) -> Result<(), RenderError> {
    match reporter {
        Reporter::Default => {
            for (name, outcome) in leaves {
                writeln!(out, "{name}: {}", outcome.verdict.state)?;
            }
            writeln!(out, "{}", set_verdict.state)?;
            Ok(())
        }
        Reporter::Quiet => Ok(()),
        Reporter::Json => {
            let runs: Vec<RunSummary> = leaves.iter().map(|(_, o)| build_summary(o)).collect();
            let summary = RunSetSummary::new(set_verdict.state, runs);
            serde_json::to_writer(&mut *out, &summary)
                .map_err(|e| RenderError::Io(std::io::Error::other(e)))?;
            writeln!(out)?;
            Ok(())
        }
        Reporter::Plugin { name, argv } => {
            // Wrap `out` so we can observe the final byte each plugin
            // wrote. A plugin that emits its own output without a
            // trailing newline would otherwise share the line with
            // the aggregated verdict, breaking the "final stdout
            // line is the aggregate verdict" contract (Copilot PR
            // #60 review).
            let mut tracked = NewlineTracker::new(out);
            for (_, outcome) in leaves {
                render_plugin(name, argv, &mut tracked, outcome)?;
            }
            if !tracked.at_line_start() {
                writeln!(&mut tracked)?;
            }
            writeln!(&mut tracked, "{}", set_verdict.state)?;
            Ok(())
        }
    }
}

/// `Write` adapter that remembers whether the last byte written was
/// `\n`. Used by the manifest + plugin reporter path to make sure the
/// aggregated verdict lands on its own line even when a plugin's
/// stdout does not end with a newline.
struct NewlineTracker<'w> {
    inner: &'w mut dyn Write,
    last_byte_was_newline: bool,
}

impl<'w> NewlineTracker<'w> {
    fn new(inner: &'w mut dyn Write) -> Self {
        // Start in the "at line start" state — an empty stream is
        // logically at the beginning of a line.
        Self {
            inner,
            last_byte_was_newline: true,
        }
    }

    fn at_line_start(&self) -> bool {
        self.last_byte_was_newline
    }
}

impl Write for NewlineTracker<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        if n > 0 {
            self.last_byte_was_newline = buf[..n].last().copied() == Some(b'\n');
        }
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Append per-failing-check assertion detail under a `fail` /
/// `inconclusive` verdict line, so an author sees *which* assertion
/// failed (and any cause) without opening `trace.jsonl`. Nothing is
/// written for a passing run (`failures` is empty). ASCII-only and
/// ANSI-free, matching the built-in reporters' plain posture.
fn write_failures(out: &mut dyn Write, failures: &[CheckFailure]) -> Result<(), RenderError> {
    for f in failures {
        writeln!(out, "  {}::{}:", f.criterion_id, f.check_id)?;
        for a in &f.assertions {
            writeln!(out, "    {}  {}", a.state, a.expr)?;
            if let Some(d) = &a.detail {
                writeln!(out, "        ({d})")?;
            }
        }
    }
    Ok(())
}

fn build_summary(o: &RunOutcome) -> RunSummary {
    let failures = o
        .failures
        .iter()
        .map(|f| CheckFailureSummary {
            criterion_id: f.criterion_id.clone(),
            check_id: f.check_id.clone(),
            assertions: f
                .assertions
                .iter()
                .map(|a| FailedAssertionSummary {
                    expr: a.expr.clone(),
                    verdict: a.state,
                    detail: a.detail.clone(),
                })
                .collect(),
        })
        .collect();
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
    .with_failures(failures)
}

/// Spawn a plugin subprocess, write the `RunSummary` JSON line to its
/// stdin, copy its stdout to `out`, and propagate any non-zero exit
/// code as a `RenderError`. Stderr is captured and inlined into the
/// error message so author plugins can fail loudly.
///
/// I/O posture:
///
/// - **Stdin is written on a helper thread.** A plugin that exits
///   without reading stdin (parse-time failure, `/bin/false`, etc.)
///   would otherwise hand us a `BrokenPipe` here that masks the
///   plugin's real `PluginExit` failure. Writing in a separate thread
///   lets the main thread proceed to `wait_with_output`, which reaps
///   the child and gives us the actual exit status + stderr.
/// - **Stdout and stderr are drained concurrently** via
///   `wait_with_output`. Reading one pipe to EOF before draining the
///   other can deadlock when a noisy plugin fills the second pipe's
///   buffer while keeping the first open.
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

    // Serialize the RunSummary up-front so the writer thread doesn't
    // need to know about it.
    let summary = build_summary(outcome);
    let line =
        serde_json::to_vec(&summary).map_err(|e| RenderError::Io(std::io::Error::other(e)))?;

    let stdin = child
        .stdin
        .take()
        .expect("stdin pipe is configured above; take() must succeed");
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut stdin = stdin;
        // Best-effort write: a `BrokenPipe` here means the plugin
        // exited without reading. Swallow it so the main thread can
        // proceed to `wait_with_output` and surface `PluginExit`
        // with the plugin's real failure. Other errors are propagated.
        match stdin.write_all(&line).and_then(|_| stdin.write_all(b"\n")) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
            Err(e) => Err(e),
        }
        // Drop closes the pipe.
    });

    // Drain stdout + stderr concurrently and wait for exit. This is
    // the standard non-deadlock idiom for capturing both streams.
    let output = child.wait_with_output().map_err(RenderError::Io)?;

    // Surface a writer-thread I/O failure only if the plugin
    // otherwise exited successfully — a PluginExit failure carries
    // more useful information for the operator.
    let writer_result = writer.join().expect("stdin writer thread panicked");

    if !output.status.success() {
        let stderr_text = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(RenderError::PluginExit {
            name: name.to_string(),
            code: output.status.code(),
            stderr: stderr_text,
        });
    }

    writer_result?;

    out.write_all(&output.stdout)?;
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

/// Match `name` against the built-in reporters (`default` / `quiet`
/// / `json`). Returns `None` if `name` is not a built-in. Split out
/// from plugin resolution so the CLI can answer for built-ins
/// without paying the cost of reading `~/.duhem/config.toml` or
/// `.duhem.toml` — a malformed plugin config must not break
/// `--reporter default` (spec on #34: built-ins are never shadowable).
pub fn resolve_built_in(name: &str) -> Option<Reporter> {
    match name {
        "default" => Some(Reporter::Default),
        "quiet" => Some(Reporter::Quiet),
        "json" => Some(Reporter::Json),
        _ => None,
    }
}

/// Resolve a non-built-in name against the plugin registry. The CLI
/// only calls this after [`resolve_built_in`] returns `None`.
pub fn resolve_plugin(
    name: &str,
    registry: &crate::reporter_config::PluginRegistry,
) -> Result<Reporter, String> {
    match registry.get(name) {
        Some(entry) => Ok(Reporter::Plugin {
            name: name.to_string(),
            argv: entry.command.clone(),
        }),
        None => Err(format!("unknown reporter: {name}")),
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
            failures: Vec::new(),
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
        // Bumped to "2" with the failure-detail addition (#125).
        assert_eq!(v["schema_version"], "2");
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
    fn resolve_built_in_returns_built_in_variant() {
        // Spec on #34: built-ins are not shadowable, and `main.rs`
        // tries `resolve_built_in` BEFORE reading any plugin config —
        // so a built-in name never even touches the registry.
        assert_eq!(resolve_built_in("default"), Some(Reporter::Default));
        assert_eq!(resolve_built_in("quiet"), Some(Reporter::Quiet));
        assert_eq!(resolve_built_in("json"), Some(Reporter::Json));
        assert_eq!(resolve_built_in("pretty"), None);
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
        let r = resolve_plugin("pretty", &registry).unwrap();
        match r {
            Reporter::Plugin { name, argv } => {
                assert_eq!(name, "pretty");
                assert_eq!(argv, vec!["duhem-reporter-pretty".to_string()]);
            }
            other => panic!("expected Plugin, got {other:?}"),
        }
    }

    #[test]
    fn resolve_plugin_unknown_name_errors() {
        let registry = PluginRegistry::default();
        let err = resolve_plugin("nope", &registry).unwrap_err();
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

    fn leaves_pair() -> (Vec<(String, RunOutcome)>, RunSetVerdict) {
        let a = outcome(VerdictState::Pass);
        let b = outcome(VerdictState::Fail);
        let set = RunSetVerdict {
            state: VerdictState::Fail,
            runs: vec![a.verdict.clone(), b.verdict.clone()],
        };
        (
            vec![("leaf-a".to_string(), a), ("leaf-b".to_string(), b)],
            set,
        )
    }

    fn capture_set(
        reporter: &Reporter,
        leaves: &[(String, RunOutcome)],
        set: &RunSetVerdict,
    ) -> String {
        let mut buf = Vec::new();
        render_set(reporter, &mut buf, leaves, set).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn render_set_default_prints_per_leaf_and_aggregate() {
        // Spec on #49: default reporter on a manifest prints one
        // `<name>: <verdict>` line per leaf, then the aggregated
        // verdict on its own line as the final line of stdout.
        let (leaves, set) = leaves_pair();
        let s = capture_set(&Reporter::Default, &leaves, &set);
        assert_eq!(s, "leaf-a: pass\nleaf-b: fail\nfail\n");
    }

    #[test]
    fn render_set_quiet_writes_nothing() {
        let (leaves, set) = leaves_pair();
        let s = capture_set(&Reporter::Quiet, &leaves, &set);
        assert_eq!(s, "");
    }

    #[test]
    fn render_set_json_emits_run_set_summary() {
        // Spec on #49: the JSON reporter on a manifest emits one
        // `RunSetSummary` line — wraps the per-leaf `RunSummary`s
        // and the aggregated verdict.
        let (leaves, set) = leaves_pair();
        let s = capture_set(&Reporter::Json, &leaves, &set);
        let trimmed = s.trim_end_matches('\n');
        assert!(!trimmed.contains('\n'), "single line: {s:?}");
        let v: serde_json::Value = serde_json::from_str(trimmed).expect("valid JSON");
        assert_eq!(v["schema_version"], "1");
        assert_eq!(v["verdict"], "fail");
        let runs = v["runs"].as_array().expect("runs array");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0]["verdict"], "pass");
        assert_eq!(runs[1]["verdict"], "fail");
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
