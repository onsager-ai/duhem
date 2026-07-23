//! Live terminal progress for `duhem run` (#299, #305).
//!
//! Subscribes to the engine's progress sink (`Engine::with_progress`,
//! the evidence tee) and renders per-criterion progress to stderr as
//! the run executes — `duhem run` is no longer silent for the whole
//! run. stdout is untouched: the post-run reporter contract stays
//! byte-identical, so CI greps and `--reporter json` consumers see
//! exactly what they saw before.
//!
//! On by default when stderr is a TTY; `--live` / `--no-live` force
//! it either way (a VD or a script capturing stderr uses `--live`).
//!
//! Two presentations (#305):
//!
//! - **TTY** — each criterion occupies exactly ONE line: the running
//!   line (`▶ AC-2 (2/3) … ui/assert-element 8s`) rewrites in place,
//!   with elapsed time and the current action ticking on it, and is
//!   REPLACED by its verdict line (`✔ AC-2 pass (1.4s)`) when the
//!   criterion settles — the final transcript shows each criterion
//!   once.
//! - **forced non-TTY** (`--live` into a capture: CI, the self-VD) —
//!   plain append lines, control-sequence-free, plus an explicit
//!   heartbeat line (`… still in cli/invoke (12s)`) once a single
//!   step has been in flight past a threshold, so a slow step is
//!   visibly alive in a log too.
//!
//! The renderer is a fold over the same evidence events the store
//! persists — no second progress model. A criterion "starts" at its
//! first event (there is no `criterion_started` on the wire; the
//! first `step_started` / assertion serves) and closes at
//! `criterion_finished` with the judge's verdict.

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::time::Duration;

use duhem_evidence::{Event, EventPayload};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;

/// What the renderer needs to know about the run up front: the
/// ordered criterion ids (for "k/n") and each check's owning
/// criterion (several event kinds carry only a `check_id`).
pub struct Plan {
    criterion_ids: Vec<String>,
    check_owner: HashMap<String, String>,
}

impl Plan {
    pub fn from_def(def: &duhem_schema::VerificationDefinition) -> Self {
        let criterion_ids = def.criteria.iter().map(|c| c.id.clone()).collect();
        let mut check_owner = HashMap::new();
        for c in &def.criteria {
            for ch in &c.checks {
                check_owner.insert(ch.id.clone(), c.id.clone());
            }
        }
        Self {
            criterion_ids,
            check_owner,
        }
    }
}

/// The `env:` narration line for an env-lifecycle payload, `None`
/// for every other event kind. Shared by the leaf renderer's fold
/// and the suite-scope narration.
fn env_line(payload: &EventPayload) -> Option<String> {
    match payload {
        EventPayload::EnvUpStarted { .. } => Some("  env: up…".to_string()),
        EventPayload::EnvUpFinished {
            exit_code,
            duration_ms,
            ..
        } => {
            let secs = *duration_ms as f64 / 1000.0;
            Some(if *exit_code == 0 {
                format!("  env: up ok ({secs:.1}s)")
            } else {
                format!("  env: up FAILED (exit {exit_code}, {secs:.1}s)")
            })
        }
        EventPayload::EnvReady { ok, elapsed_ms, .. } => {
            let secs = *elapsed_ms as f64 / 1000.0;
            Some(if *ok {
                format!("  env: ready ({secs:.1}s)")
            } else {
                format!("  env: NOT ready ({secs:.1}s)")
            })
        }
        EventPayload::EnvDownStarted { .. } => Some("  env: down…".to_string()),
        EventPayload::EnvDownFinished {
            exit_code,
            duration_ms,
            ..
        } => {
            let secs = *duration_ms as f64 / 1000.0;
            Some(if *exit_code == 0 {
                format!("  env: down ({secs:.1}s)")
            } else {
                format!("  env: down FAILED (exit {exit_code}, {secs:.1}s)")
            })
        }
        _ => None,
    }
}

/// Narrate one suite-environment event to stderr (#305 A) — the env
/// lifecycle subset of the fold, stateless and synchronous so
/// `run_cmd` can drive it inline and keep suite `env:` lines
/// deterministically ordered against its own stderr lines (headers,
/// live links). Every other event kind is ignored.
pub fn narrate_env_event_to_stderr(evt: &Event) {
    if let Some(text) = env_line(&evt.payload) {
        eprintln!("{text}");
    }
}

/// How often the renderer wakes without an event — the TTY redraw
/// cadence and the granularity of non-TTY heartbeat checks.
const TICK_PERIOD: Duration = Duration::from_secs(1);

/// Presentation posture + heartbeat cadence (#305).
#[derive(Clone, Copy)]
pub struct RenderConfig {
    /// In-place single-line rewriting. Only a real terminal gets it:
    /// when `--live` forces rendering into a capture, control
    /// sequences would garble the log, so `false` keeps plain
    /// append-only lines.
    pub tty: bool,
    /// Non-TTY: how long one step must be in flight before the first
    /// `… still in <uses>` heartbeat line.
    pub heartbeat_threshold: Duration,
    /// Non-TTY: repeat cadence after the first heartbeat.
    pub heartbeat_period: Duration,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            tty: false,
            heartbeat_threshold: Duration::from_secs(10),
            heartbeat_period: Duration::from_secs(10),
        }
    }
}

impl RenderConfig {
    /// The production config: presentation keyed off whether stderr
    /// is a terminal — independent of *why* live mode is on, so a
    /// `--live` in CI still gets clean append lines.
    pub fn detect() -> Self {
        Self {
            tty: std::io::stderr().is_terminal(),
            ..Self::default()
        }
    }
}

/// Drain the progress channel to stderr until `run_finished` (or the
/// channel closes). Spawned alongside the engine's run future.
pub async fn render_to_stderr(rx: UnboundedReceiver<Event>, plan: Plan, cfg: RenderConfig) {
    let mut err = std::io::stderr();
    render(rx, plan, cfg, &mut err).await;
}

/// The fold itself, writer-generic for tests. Write errors are
/// ignored — progress is advisory and must never disturb the run.
pub async fn render<W: Write>(
    mut rx: UnboundedReceiver<Event>,
    plan: Plan,
    cfg: RenderConfig,
    out: &mut W,
) {
    let mut r = Renderer {
        out,
        cfg,
        n: plan.criterion_ids.len(),
        check_owner: plan.check_owner,
        started: HashMap::new(),
        running: None,
        line_open: false,
    };
    let mut tick = tokio::time::interval(TICK_PERIOD);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            evt = rx.recv() => match evt {
                None => break,
                Some(evt) => {
                    if r.event(evt) {
                        break;
                    }
                }
            },
            _ = tick.tick() => r.tick(),
        }
    }
    // Never leave an unterminated in-place line behind (engine-error
    // abort, channel close): the next writer would glue onto it.
    r.close();
}

/// The criterion currently narrating, with the monotonic clocks that
/// drive its elapsed display and heartbeat cadence. (Verdict
/// durations use evidence `ts` instead — they must match the stored
/// trace, not this process's clocks.)
struct Running {
    criterion_id: String,
    ordinal: usize,
    since: Instant,
    step_uses: Option<String>,
    step_since: Instant,
    beats: u32,
}

struct Renderer<'a, W: Write> {
    out: &'a mut W,
    cfg: RenderConfig,
    n: usize,
    check_owner: HashMap<String, String>,
    /// criterion id → (ordinal shown at start, first-event timestamp)
    started: HashMap<String, (usize, chrono::DateTime<chrono::Utc>)>,
    running: Option<Running>,
    /// TTY: an in-place running line is currently displayed,
    /// unterminated.
    line_open: bool,
}

impl<W: Write> Renderer<'_, W> {
    /// Fold one event; `true` means the run is over.
    fn event(&mut self, evt: Event) -> bool {
        let ts = evt.ts;
        if let Some(text) = env_line(&evt.payload) {
            self.line(&text);
            return false;
        }
        match &evt.payload {
            EventPayload::StepStarted {
                criterion_id, uses, ..
            } => {
                self.begin(criterion_id, ts);
                if let Some(run) = &mut self.running
                    && run.criterion_id == *criterion_id
                {
                    run.step_uses = Some(uses.clone());
                    run.step_since = Instant::now();
                    run.beats = 0;
                }
                if self.cfg.tty {
                    self.draw_running();
                }
            }
            EventPayload::StepFinished { .. } => {
                if let Some(run) = &mut self.running {
                    run.step_uses = None;
                    run.beats = 0;
                }
                if self.cfg.tty {
                    self.draw_running();
                }
            }
            EventPayload::AssertionEvaluated { check_id, .. }
            | EventPayload::CheckFinished { check_id, .. } => {
                if let Some(cid) = self.check_owner.get(check_id) {
                    let owner = cid.clone();
                    self.begin(&owner, ts);
                }
            }
            EventPayload::CriterionFinished {
                criterion_id,
                verdict,
            } => {
                // A criterion whose checks were all filtered out (or
                // stepless-and-instant) may finish without a start
                // line; give it one so every verdict line has context.
                self.begin(criterion_id, ts);
                let mark = match verdict {
                    duhem_judge::VerdictState::Pass => "✔",
                    duhem_judge::VerdictState::Fail => "✘",
                    duhem_judge::VerdictState::Inconclusive(_) => "◐",
                };
                let secs = self
                    .started
                    .get(criterion_id)
                    .map(|(_, t0)| (ts - *t0).num_milliseconds() as f64 / 1000.0)
                    .unwrap_or(0.0);
                // One final line per criterion (#305 E): on a TTY
                // this REPLACES the in-place running line.
                self.line(&format!("{mark} {criterion_id} {verdict} ({secs:.1}s)"));
                self.running = None;
            }
            EventPayload::RunFinished { .. } => return true,
            _ => {}
        }
        false
    }

    /// Periodic wake with no event: redraw the running line (TTY) or
    /// consider a heartbeat line (non-TTY).
    fn tick(&mut self) {
        if self.running.is_none() {
            return;
        }
        if self.cfg.tty {
            self.draw_running();
            return;
        }
        let Some(run) = &self.running else { return };
        let Some(uses) = run.step_uses.clone() else {
            return;
        };
        let elapsed = run.step_since.elapsed();
        if elapsed < self.cfg.heartbeat_threshold + self.cfg.heartbeat_period * run.beats {
            return;
        }
        if let Some(run) = &mut self.running {
            run.beats += 1;
        }
        let _ = writeln!(self.out, "  … still in {uses} ({}s)", elapsed.as_secs());
        let _ = self.out.flush();
    }

    /// Register a criterion the first time it is seen (idempotent)
    /// and open its running line.
    fn begin(&mut self, criterion_id: &str, ts: chrono::DateTime<chrono::Utc>) {
        if self.started.contains_key(criterion_id) {
            return;
        }
        let k = self.started.len() + 1;
        self.started.insert(criterion_id.to_string(), (k, ts));
        self.running = Some(Running {
            criterion_id: criterion_id.to_string(),
            ordinal: k,
            since: Instant::now(),
            step_uses: None,
            step_since: Instant::now(),
            beats: 0,
        });
        if self.cfg.tty {
            self.draw_running();
        } else {
            let n = self.n;
            let _ = writeln!(self.out, "▶ {criterion_id} ({k}/{n})…");
        }
    }

    /// TTY: (re)draw the running criterion's single in-place line —
    /// carriage return + erase-line, no trailing newline. The verdict
    /// line later replaces it via [`Renderer::line`].
    fn draw_running(&mut self) {
        let Some(run) = &self.running else { return };
        let head = format!("▶ {} ({}/{})", run.criterion_id, run.ordinal, self.n);
        let text = match &run.step_uses {
            Some(uses) => format!("{head} … {uses} {}s", run.step_since.elapsed().as_secs()),
            None => {
                let secs = run.since.elapsed().as_secs();
                if secs == 0 {
                    format!("{head}…")
                } else {
                    format!("{head}… {secs}s")
                }
            }
        };
        let _ = write!(self.out, "\r\x1b[2K{text}");
        let _ = self.out.flush();
        self.line_open = true;
    }

    /// Write one durable line. On a TTY an open running line is
    /// erased first — the durable line takes its place (the running
    /// line redraws on the next tick if its criterion is still
    /// going).
    fn line(&mut self, text: &str) {
        if self.cfg.tty && self.line_open {
            let _ = write!(self.out, "\r\x1b[2K");
            self.line_open = false;
        }
        let _ = writeln!(self.out, "{text}");
        let _ = self.out.flush();
    }

    /// Terminate a still-open in-place line.
    fn close(&mut self) {
        if self.line_open {
            let _ = writeln!(self.out);
            let _ = self.out.flush();
            self.line_open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duhem_evidence::EventPayload;
    use duhem_judge::VerdictState;

    fn plan() -> Plan {
        let def = duhem_schema::VerificationDefinition::from_yaml_str(
            r#"
verification: t
criteria:
  - id: AC-1
    description: one
    checks:
      - id: AC-1.1
        assertions: ["true"]
  - id: AC-2
    description: two
    checks:
      - id: AC-2.1
        assertions: ["true"]
"#,
        )
        .expect("parse");
        Plan::from_def(&def)
    }

    fn evt(seq: u64, offset_ms: i64, payload: EventPayload) -> Event {
        Event {
            seq,
            ts: chrono::DateTime::parse_from_rfc3339("2026-07-23T00:00:00Z")
                .unwrap()
                .to_utc()
                + chrono::Duration::milliseconds(offset_ms),
            payload,
        }
    }

    /// Append-mode config (the forced-`--live`-without-a-TTY path
    /// the self-VD exercises).
    fn append_cfg() -> RenderConfig {
        RenderConfig::default()
    }

    fn tty_cfg() -> RenderConfig {
        RenderConfig {
            tty: true,
            ..RenderConfig::default()
        }
    }

    async fn rendered(events: Vec<Event>, cfg: RenderConfig) -> String {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        for e in events {
            tx.send(e).unwrap();
        }
        drop(tx);
        let mut out = Vec::new();
        render(rx, plan(), cfg, &mut out).await;
        String::from_utf8(out).unwrap()
    }

    /// Interpret `\r` + erase-line the way a terminal would: what
    /// survives on screen per row.
    fn visible(raw: &str) -> Vec<String> {
        raw.split('\n')
            .map(|row| {
                let after_erase = row.rsplit("\u{1b}[2K").next().unwrap_or(row);
                after_erase
                    .rsplit('\r')
                    .next()
                    .unwrap_or(after_erase)
                    .to_string()
            })
            .filter(|row| !row.is_empty())
            .collect()
    }

    #[tokio::test]
    async fn per_criterion_lines_with_ordinals_verdicts_and_durations() {
        let out = rendered(
            vec![
                evt(0, 0, duhem_evidence::run_started("t", Default::default())),
                evt(
                    1,
                    100,
                    EventPayload::AssertionEvaluated {
                        check_id: "AC-1.1".into(),
                        assertion_index: 0,
                        state: VerdictState::Pass,
                        detail: None,
                        step_index: None,
                    },
                ),
                evt(
                    2,
                    1600,
                    EventPayload::CriterionFinished {
                        criterion_id: "AC-1".into(),
                        verdict: VerdictState::Pass,
                    },
                ),
                evt(
                    3,
                    1700,
                    EventPayload::AssertionEvaluated {
                        check_id: "AC-2.1".into(),
                        assertion_index: 0,
                        state: VerdictState::Fail,
                        detail: None,
                        step_index: None,
                    },
                ),
                evt(
                    4,
                    1900,
                    EventPayload::CriterionFinished {
                        criterion_id: "AC-2".into(),
                        verdict: VerdictState::Fail,
                    },
                ),
                evt(
                    5,
                    2000,
                    EventPayload::RunFinished {
                        verdict: VerdictState::Fail,
                    },
                ),
            ],
            append_cfg(),
        )
        .await;
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines,
            vec![
                "▶ AC-1 (1/2)…",
                "✔ AC-1 pass (1.5s)",
                "▶ AC-2 (2/2)…",
                "✘ AC-2 fail (0.2s)",
            ]
        );
    }

    #[tokio::test]
    async fn environment_lifecycle_is_narrated() {
        let out = rendered(
            vec![
                evt(0, 0, duhem_evidence::run_started("t", Default::default())),
                evt(
                    1,
                    10,
                    EventPayload::EnvUpStarted {
                        command: "./up.sh".into(),
                    },
                ),
                evt(
                    2,
                    310,
                    EventPayload::EnvUpFinished {
                        exit_code: 0,
                        duration_ms: 300,
                        stdout_blob_sha256: None,
                        stderr_blob_sha256: None,
                    },
                ),
                evt(
                    3,
                    2810,
                    EventPayload::EnvReady {
                        probe_kind: "http".into(),
                        ok: true,
                        elapsed_ms: 2500,
                    },
                ),
                evt(
                    4,
                    2900,
                    EventPayload::EnvDownStarted {
                        command: "./down.sh".into(),
                    },
                ),
                evt(
                    5,
                    3000,
                    EventPayload::EnvDownFinished {
                        exit_code: 0,
                        duration_ms: 100,
                        stdout_blob_sha256: None,
                        stderr_blob_sha256: None,
                    },
                ),
                evt(
                    6,
                    3100,
                    EventPayload::RunFinished {
                        verdict: VerdictState::Pass,
                    },
                ),
            ],
            append_cfg(),
        )
        .await;
        assert!(out.contains("env: up…"), "{out}");
        assert!(out.contains("env: up ok (0.3s)"), "{out}");
        assert!(out.contains("env: ready (2.5s)"), "{out}");
        assert!(out.contains("env: down…"), "{out}");
        assert!(out.contains("env: down (0.1s)"), "{out}");
    }

    #[tokio::test]
    async fn stream_ends_at_run_finished_even_with_sender_alive() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tx.send(evt(
            0,
            0,
            EventPayload::RunFinished {
                verdict: VerdictState::Pass,
            },
        ))
        .unwrap();
        // tx deliberately kept alive: render must return at
        // run_finished, not wait for channel close.
        let mut out = Vec::new();
        render(rx, plan(), append_cfg(), &mut out).await;
        drop(tx);
    }

    /// #305 E: on a TTY each criterion is exactly ONE line — the
    /// running line is erased (`\r` + `ESC[2K`) and replaced by its
    /// verdict line, so the final transcript shows each criterion
    /// once. Paused clock so elapsed displays are deterministic.
    #[tokio::test(start_paused = true)]
    async fn tty_running_line_is_replaced_by_the_verdict_line() {
        let out = rendered(
            vec![
                evt(0, 0, duhem_evidence::run_started("t", Default::default())),
                evt(
                    1,
                    100,
                    EventPayload::StepStarted {
                        criterion_id: "AC-1".into(),
                        check_id: "AC-1.1".into(),
                        step_index: 0,
                        uses: "cli/invoke".into(),
                        layer: None,
                        with: Default::default(),
                    },
                ),
                evt(
                    2,
                    1600,
                    EventPayload::CriterionFinished {
                        criterion_id: "AC-1".into(),
                        verdict: VerdictState::Pass,
                    },
                ),
                evt(
                    3,
                    1700,
                    EventPayload::CriterionFinished {
                        criterion_id: "AC-2".into(),
                        verdict: VerdictState::Fail,
                    },
                ),
                evt(
                    4,
                    1800,
                    EventPayload::RunFinished {
                        verdict: VerdictState::Fail,
                    },
                ),
            ],
            tty_cfg(),
        )
        .await;
        // Raw control-sequence contract: a running line was drawn
        // (with the in-flight action on it), then erased and replaced
        // by the verdict line.
        assert!(out.contains("▶ AC-1 (1/2)"), "{out:?}");
        assert!(out.contains("… cli/invoke 0s"), "{out:?}");
        assert!(out.contains("\r\u{1b}[2K✔ AC-1 pass (1.5s)\n"), "{out:?}");
        // What a terminal ultimately shows: one line per criterion,
        // no start/finish duplication.
        assert_eq!(
            visible(&out),
            vec!["✔ AC-1 pass (1.5s)", "✘ AC-2 fail (0.0s)"]
        );
    }

    /// #305 C: without a TTY, a step in flight past the threshold
    /// appends `… still in <uses> (Ns)` heartbeat lines, repeating
    /// each period. Paused clock — the interval ticks auto-advance,
    /// no real sleeps.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_lines_append_past_threshold_without_a_tty() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tx.send(evt(
            0,
            0,
            duhem_evidence::run_started("t", Default::default()),
        ))
        .unwrap();
        tx.send(evt(
            1,
            10,
            EventPayload::StepStarted {
                criterion_id: "AC-1".into(),
                check_id: "AC-1.1".into(),
                step_index: 0,
                uses: "cli/invoke".into(),
                layer: None,
                with: Default::default(),
            },
        ))
        .unwrap();
        let cfg = RenderConfig {
            tty: false,
            heartbeat_threshold: Duration::from_secs(2),
            heartbeat_period: Duration::from_secs(2),
        };
        let handle = tokio::spawn(async move {
            let mut out = Vec::new();
            render(rx, plan(), cfg, &mut out).await;
            String::from_utf8(out).unwrap()
        });
        // The step stays in flight for 5s of (paused, auto-advanced)
        // time: first beat at 2s, second at 4s.
        tokio::time::sleep(Duration::from_secs(5)).await;
        tx.send(evt(
            2,
            5100,
            EventPayload::CriterionFinished {
                criterion_id: "AC-1".into(),
                verdict: VerdictState::Pass,
            },
        ))
        .unwrap();
        tx.send(evt(
            3,
            5200,
            EventPayload::RunFinished {
                verdict: VerdictState::Pass,
            },
        ))
        .unwrap();
        let out = handle.await.unwrap();
        assert!(out.contains("… still in cli/invoke (2s)"), "{out}");
        assert!(out.contains("… still in cli/invoke (4s)"), "{out}");
        assert!(out.contains("✔ AC-1 pass"), "{out}");
    }
}
