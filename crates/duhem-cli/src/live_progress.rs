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
//! - **TTY** — a width-bounded live board redraws in place. It shows
//!   every criterion, expands started criteria into check and step
//!   branches, retains completed pass/fail state, and animates the
//!   active leaf with a spinner and fractional timeout bar
//!   (`⠹ █████▎──────────  26s/60s`). Semantic color is disabled by
//!   `NO_COLOR` and `TERM=dumb`.
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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

mod tty;

use tty::TtyBoard;

/// What the renderer needs to know about the run up front: the
/// ordered criterion ids (for "k/n") and each check's owning
/// criterion (several event kinds carry only a `check_id`).
pub struct Plan {
    verification: String,
    criterion_ids: Vec<String>,
    check_owner: HashMap<String, String>,
    checks: HashMap<String, CheckPlan>,
    criteria: Vec<CriterionPlan>,
}

/// Presentation-only metadata collected from the authored definition.
/// It stays in the CLI: evidence remains the runtime's generic event
/// stream, while the terminal can still say where an event sits in a
/// check and use the human check summary when one exists.
#[derive(Clone)]
struct CheckPlan {
    id: String,
    description: Option<String>,
    step_count: usize,
}

#[derive(Clone)]
struct CriterionPlan {
    id: String,
    description: String,
    checks: Vec<CheckPlan>,
}

impl Plan {
    pub fn from_def(def: &duhem_schema::VerificationDefinition) -> Self {
        let criterion_ids = def.criteria.iter().map(|c| c.id.clone()).collect();
        let mut check_owner = HashMap::new();
        let mut checks = HashMap::new();
        let mut criteria = Vec::new();
        for c in &def.criteria {
            let mut criterion_checks = Vec::new();
            for ch in &c.checks {
                check_owner.insert(ch.id.clone(), c.id.clone());
                let check = CheckPlan {
                    id: ch.id.clone(),
                    description: ch.description.clone(),
                    step_count: ch.steps.len(),
                };
                checks.insert(ch.id.clone(), check.clone());
                criterion_checks.push(check);
            }
            criteria.push(CriterionPlan {
                id: c.id.clone(),
                description: c.description.clone(),
                checks: criterion_checks,
            });
        }
        Self {
            verification: def.verification.clone(),
            criterion_ids,
            check_owner,
            checks,
            criteria,
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

/// How often the renderer wakes without an event — the TTY animation
/// cadence and the granularity of non-TTY heartbeat checks. Evidence
/// events still render immediately.
const TICK_PERIOD: Duration = Duration::from_millis(250);

/// Text-presentation status vocabulary. These deliberately avoid
/// emoji-capable dingbats such as `✔` / `✘`; every indicator occupies
/// exactly one terminal cell under Unicode's terminal-width rules.
pub(super) const INDICATOR_PASS: &str = "✓";
pub(super) const INDICATOR_FAIL: &str = "✗";
pub(super) const INDICATOR_INCONCLUSIVE: &str = "◐";
pub(super) const INDICATOR_PENDING: &str = "○";
pub(super) const INDICATOR_ACTIVE: &str = "›";

/// Presentation posture + heartbeat cadence (#305).
#[derive(Clone, Copy)]
pub struct RenderConfig {
    /// In-place single-line rewriting. Only a real terminal gets it:
    /// when `--live` forces rendering into a capture, control
    /// sequences would garble the log, so `false` keeps plain
    /// append-only lines.
    pub tty: bool,
    /// ANSI semantic color. Kept separate from `tty` so tests and
    /// embedding callers can request in-place redraws without escape
    /// codes.
    pub color: bool,
    /// Maximum terminal columns used by the in-place renderer. A live
    /// line must never wrap: `CSI 2K` clears one physical row only.
    pub terminal_width: usize,
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
            color: false,
            terminal_width: 80,
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
        let tty = std::io::stderr().is_terminal();
        let color = tty
            && std::env::var_os("NO_COLOR").is_none()
            && !std::env::var("TERM").is_ok_and(|term| term.eq_ignore_ascii_case("dumb"));
        Self {
            tty,
            color,
            terminal_width: std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|width: &usize| *width >= 20)
                .unwrap_or(80),
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
    let board = TtyBoard::new(
        plan.verification,
        plan.criteria,
        cfg.terminal_width,
        cfg.color,
    );
    let mut r = Renderer {
        out,
        cfg,
        n: plan.criterion_ids.len(),
        check_owner: plan.check_owner,
        checks: plan.checks,
        board,
        started: HashMap::new(),
        running: None,
        line_open: false,
        last_board_lines: Vec::new(),
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
    check_id: Option<String>,
    check_description: Option<String>,
    step_index: Option<u32>,
    step_count: usize,
    expectation: Option<String>,
    timeout: Option<Duration>,
    step_since: Instant,
    beats: u32,
}

struct Renderer<'a, W: Write> {
    out: &'a mut W,
    cfg: RenderConfig,
    n: usize,
    check_owner: HashMap<String, String>,
    checks: HashMap<String, CheckPlan>,
    board: TtyBoard,
    /// criterion id → (ordinal shown at start, first-event timestamp)
    started: HashMap<String, (usize, chrono::DateTime<chrono::Utc>)>,
    running: Option<Running>,
    /// TTY: an in-place running line is currently displayed,
    /// unterminated.
    line_open: bool,
    /// Fully rendered rows from the last TTY board refresh. Event
    /// folds may request a redraw without changing visible state; do
    /// not touch the terminal in that case.
    last_board_lines: Vec<String>,
}

impl<W: Write> Renderer<'_, W> {
    /// Fold one event; `true` means the run is over.
    fn event(&mut self, evt: Event) -> bool {
        let ts = evt.ts;
        if let Some(text) = env_line(&evt.payload) {
            self.line(&text);
            return false;
        }
        if self.cfg.tty {
            return self.event_tty(evt);
        }
        match &evt.payload {
            EventPayload::StepStarted {
                criterion_id,
                check_id,
                step_index,
                uses,
                with,
                ..
            } => {
                self.begin(criterion_id, ts);
                if let Some(run) = &mut self.running
                    && run.criterion_id == *criterion_id
                {
                    run.step_uses = Some(uses.clone());
                    run.check_id = Some(check_id.clone());
                    if let Some(check) = self.checks.get(check_id) {
                        run.check_description = check.description.clone();
                        run.step_count = check.step_count;
                    }
                    run.step_index = Some(*step_index);
                    run.expectation = expectation(with);
                    run.timeout = timeout(with);
                    run.step_since = Instant::now();
                    run.beats = 0;
                }
                if self.cfg.tty {
                    self.draw_running();
                }
            }
            EventPayload::StepFinished { outcome, .. } => {
                if self.cfg.tty {
                    self.finish_step(outcome);
                } else if let Some(run) = &mut self.running {
                    run.step_uses = None;
                    run.beats = 0;
                }
            }
            EventPayload::AssertionEvaluated { check_id, .. } => {
                if let Some(cid) = self.check_owner.get(check_id) {
                    let owner = cid.clone();
                    self.begin(&owner, ts);
                }
            }
            EventPayload::CheckFinished { check_id, verdict } => {
                if let Some(cid) = self.check_owner.get(check_id) {
                    let owner = cid.clone();
                    self.begin(&owner, ts);
                }
                if self.cfg.tty {
                    self.line(&format!(
                        "  └─ {} {check_id} {verdict}",
                        verdict_mark(verdict)
                    ));
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
                let mark = verdict_mark(verdict);
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

    fn event_tty(&mut self, evt: Event) -> bool {
        let (done, redraw) = match &evt.payload {
            EventPayload::StepStarted {
                criterion_id,
                check_id,
                step_index,
                uses,
                with,
                ..
            } => {
                self.board
                    .start_step(criterion_id, check_id, *step_index, uses, with);
                (false, true)
            }
            EventPayload::StepFinished { outcome, .. } => {
                self.board.finish_step(outcome);
                (false, true)
            }
            EventPayload::AssertionEvaluated {
                check_id,
                step_index,
                state,
                ..
            } => {
                self.board.assertion(check_id, *step_index, state);
                (false, true)
            }
            EventPayload::CheckFinished { check_id, verdict } => {
                self.board.finish_check(check_id, verdict);
                (false, true)
            }
            EventPayload::CriterionFinished {
                criterion_id,
                verdict,
            } => {
                self.board.finish_criterion(criterion_id, verdict);
                (false, true)
            }
            EventPayload::RunFinished { .. } => (true, true),
            _ => (false, false),
        };
        if redraw {
            self.draw_board();
        }
        done
    }

    /// Periodic wake with no event: redraw the running line (TTY) or
    /// consider a heartbeat line (non-TTY).
    fn tick(&mut self) {
        if self.running.is_none() {
            if self.cfg.tty && self.board.is_active() {
                self.draw_board();
            }
            return;
        }
        if self.cfg.tty {
            self.draw_board();
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
            check_id: None,
            check_description: None,
            step_index: None,
            step_count: 0,
            expectation: None,
            timeout: None,
            step_since: Instant::now(),
            beats: 0,
        });
        if self.cfg.tty {
            let n = self.n;
            self.line(&format!("{INDICATOR_ACTIVE} {criterion_id} ({k}/{n})"));
        } else {
            let n = self.n;
            let _ = writeln!(self.out, "{INDICATOR_ACTIVE} {criterion_id} ({k}/{n})…");
        }
    }

    /// TTY: (re)draw the running criterion's single in-place line —
    /// carriage return + erase-line, no trailing newline. The verdict
    /// line later replaces it via [`Renderer::line`].
    fn draw_running(&mut self) {
        let Some(run) = &self.running else { return };
        let head = format!(
            "{INDICATOR_ACTIVE} {} ({}/{})",
            run.criterion_id, run.ordinal, self.n
        );
        let text = match &run.step_uses {
            Some(uses) => {
                let step = run
                    .step_index
                    .map(|index| match run.step_count {
                        0 => format!("step {}", index + 1),
                        total => format!("step {}/{}", index + 1, total),
                    })
                    .unwrap_or_else(|| "step".to_string());
                let check = match (run.check_id.as_deref(), run.check_description.as_deref()) {
                    (Some(id), Some(description)) => {
                        format!("{id} ({})", shorten(description, 36))
                    }
                    (Some(id), None) => id.to_string(),
                    _ => "check".to_string(),
                };
                let expectation = run
                    .expectation
                    .as_deref()
                    .map(|value| format!(" • {value}"))
                    .unwrap_or_default();
                let elapsed = run.step_since.elapsed();
                let timer = match run.timeout {
                    Some(timeout) => format!(
                        "  {}  {}s / {}s",
                        progress_bar(elapsed, timeout),
                        elapsed.as_secs(),
                        timeout.as_secs()
                    ),
                    None => format!(" {}s", elapsed.as_secs()),
                };
                let full = format!(
                    "  └─ {INDICATOR_ACTIVE} {check} · {step} · {uses}{expectation}{timer}"
                );
                let without_summary = format!(
                    "  └─ {INDICATOR_ACTIVE} {} · {step} · {uses}{expectation}{timer}",
                    run.check_id.as_deref().unwrap_or("check")
                );
                // This compact shape deliberately preserves the useful
                // live facts (check, position, action, timer/bar) on an
                // 80-column terminal. The longer variants are used only
                // when they fit a single physical terminal row.
                let compact_step = run
                    .step_index
                    .map(|index| match run.step_count {
                        0 => (index + 1).to_string(),
                        total => format!("{}/{}", index + 1, total),
                    })
                    .unwrap_or_else(|| "step".to_string());
                let compact = format!(
                    "  └─ {INDICATOR_ACTIVE} {} · {compact_step} {uses}{timer}",
                    run.check_id.as_deref().unwrap_or("check"),
                );
                [full, without_summary, compact.clone()]
                    .into_iter()
                    .find(|candidate| display_width(candidate) <= self.cfg.terminal_width)
                    .unwrap_or_else(|| shorten_to_width(&compact, self.cfg.terminal_width))
            }
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

    fn draw_board(&mut self) {
        let lines = self.board.lines();
        if lines == self.last_board_lines {
            return;
        }
        let old_height = self.board.frame_height();
        let hide_cursor = !self.board.cursor_hidden();
        let frame = compose_board_frame(&lines, old_height, hide_cursor);

        // One complete frame per write keeps terminals from painting
        // the intermediate clear/rewrite states that caused flashing.
        if self.out.write_all(frame.as_bytes()).is_ok() {
            let _ = self.out.flush();
            self.board.set_cursor_hidden(true);
            self.board.set_frame_height(lines.len());
            self.last_board_lines = lines;
        }
    }

    /// Turn the active redraw into a durable tree branch. This reports
    /// action completion only; the later check verdict remains the
    /// authoritative pass/fail judgment for assertion actions.
    fn finish_step(&mut self, outcome: &duhem_evidence::StepOutcome) {
        let Some(run) = &self.running else { return };
        let mark = match outcome {
            duhem_evidence::StepOutcome::Ok => INDICATOR_PASS,
            duhem_evidence::StepOutcome::Error => INDICATOR_FAIL,
            duhem_evidence::StepOutcome::Timeout => INDICATOR_INCONCLUSIVE,
        };
        let check = run.check_id.as_deref().unwrap_or("check");
        let step = run
            .step_index
            .map(|index| match run.step_count {
                0 => format!("step {}", index + 1),
                total => format!("step {}/{}", index + 1, total),
            })
            .unwrap_or_else(|| "step".to_string());
        let uses = run.step_uses.as_deref().unwrap_or("action");
        let elapsed = run.step_since.elapsed().as_secs_f64();
        self.line(&format!(
            "  ├─ {mark} {check} · {step} · {uses} ({elapsed:.1}s)"
        ));
        if let Some(run) = &mut self.running {
            run.step_uses = None;
            run.beats = 0;
        }
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
        if self.board.cursor_hidden() {
            let _ = write!(self.out, "\x1b[?25h");
            let _ = self.out.flush();
            self.board.set_cursor_hidden(false);
        }
    }
}

fn verdict_mark(verdict: &duhem_judge::VerdictState) -> &'static str {
    match verdict {
        duhem_judge::VerdictState::Pass => INDICATOR_PASS,
        duhem_judge::VerdictState::Fail => INDICATOR_FAIL,
        duhem_judge::VerdictState::Inconclusive(_) => INDICATOR_INCONCLUSIVE,
    }
}

/// Build a complete cursor move + row rewrite transaction. Keeping
/// composition pure makes it possible to verify that a refresh reaches
/// the writer atomically.
fn compose_board_frame(lines: &[String], old_height: usize, hide_cursor: bool) -> String {
    let surplus = old_height.saturating_sub(lines.len());
    let row_bytes = lines.iter().map(|line| line.len() + 6).sum::<usize>();
    let mut frame = String::with_capacity(row_bytes + surplus * 6 + 32);
    if hide_cursor {
        frame.push_str("\x1b[?25l");
    }
    if old_height > 0 {
        frame.push_str(&format!("\x1b[{old_height}A"));
    }
    for line in lines {
        frame.push_str("\r\x1b[2K");
        frame.push_str(line);
        frame.push('\n');
    }
    if surplus > 0 {
        for _ in 0..surplus {
            frame.push_str("\r\x1b[2K\n");
        }
        frame.push_str(&format!("\x1b[{surplus}A"));
    }
    frame
}

/// Render the useful, non-sensitive part of an action's immediate goal.
/// The action name remains the source of truth; this is only the small
/// operator-facing cue that explains why a long wait is happening.
fn expectation(with: &std::collections::BTreeMap<String, serde_json::Value>) -> Option<String> {
    if let Some(expected) = with.get("expected").and_then(serde_json::Value::as_str) {
        return Some(format!("expect {expected}"));
    }
    if let Some(state) = with.get("state").and_then(serde_json::Value::as_str) {
        return Some(format!("wait for {state}"));
    }
    None
}

/// `within:` is shared by the action families and reaches evidence as a
/// resolved JSON scalar. Keep the parser deliberately narrow: an unknown
/// value merely omits the bar, never affects execution or rendering.
fn timeout(with: &std::collections::BTreeMap<String, serde_json::Value>) -> Option<Duration> {
    let raw = with.get("within")?;
    if let Some(ms) = raw.as_u64() {
        return Some(Duration::from_millis(ms));
    }
    let raw = raw.as_str()?.trim();
    let (number, multiplier) = if let Some(value) = raw.strip_suffix("ms") {
        (value, 1_u64)
    } else if let Some(value) = raw.strip_suffix('s') {
        (value, 1_000)
    } else if let Some(value) = raw.strip_suffix('m') {
        (value, 60_000)
    } else {
        return None;
    };
    number
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|value| value.checked_mul(multiplier))
        .map(Duration::from_millis)
}

/// A smooth sixteen-cell timeout bar. Eighth-cell glyphs keep short waits
/// visibly moving without making the bar jitter in width.
fn progress_bar(elapsed: Duration, timeout: Duration) -> String {
    const WIDTH: usize = 16;
    const PARTIAL: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];
    let total = timeout.as_millis().max(1);
    let units = ((elapsed.as_millis().min(total) * (WIDTH * 8) as u128) / total) as usize;
    let full = units / 8;
    let remainder = units % 8;
    let partial = PARTIAL[remainder];
    let track = WIDTH - full - usize::from(remainder > 0);
    format!("{}{}{}", "█".repeat(full), partial, "─".repeat(track))
}

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner(elapsed: Duration) -> &'static str {
    let frame = (elapsed.as_millis() / TICK_PERIOD.as_millis()) as usize;
    SPINNER[frame % SPINNER.len()]
}

fn colorize(text: &str) -> String {
    const RESET: &str = "\x1b[0m";
    const CYAN: &str = "\x1b[36m";
    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";
    const YELLOW: &str = "\x1b[33m";
    const DIM: &str = "\x1b[2m";
    const HEADER: &str = "\x1b[1;36m";

    let header = SPINNER.iter().any(|frame| text.starts_with(frame)) && text.contains(" Duhem  ");
    if header {
        return format!("{HEADER}{text}{RESET}");
    }

    let mut out = String::with_capacity(text.len() + 32);
    for ch in text.chars() {
        let style = match ch {
            '✓' => Some(GREEN),
            '✗' => Some(RED),
            '◐' => Some(YELLOW),
            '○' | '│' | '├' | '└' | '─' => Some(DIM),
            '█' | '▏' | '▎' | '▍' | '▌' | '▋' | '▊' | '▉' => Some(CYAN),
            ch if SPINNER.iter().any(|frame| frame.starts_with(ch)) => Some(CYAN),
            _ => None,
        };
        if let Some(style) = style {
            out.push_str(style);
            out.push(ch);
            out.push_str(RESET);
        } else {
            out.push(ch);
        }
    }
    out
}

/// Keep an authored check summary useful on a narrow terminal without
/// slicing a UTF-8 code point or letting the redraw consume the whole row.
fn shorten(text: &str, width: usize) -> String {
    let mut chars = text.chars();
    let prefix: String = chars.by_ref().take(width).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

/// ANSI escapes are not part of these strings. Use Unicode terminal-cell
/// width so text indicators, braille animation, CJK content, and ASCII
/// all share the same fit calculation.
fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn shorten_to_width(text: &str, width: usize) -> String {
    if display_width(text) <= width {
        return text.to_string();
    }
    let ellipsis_width = display_width("…");
    let budget = width.saturating_sub(ellipsis_width);
    let mut used = 0;
    let mut out = String::new();
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > budget {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    format!("{out}…")
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
        description: one check
        steps:
          - uses: cli/invoke
          - uses: cli/invoke
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
            terminal_width: 200,
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

    #[derive(Default)]
    struct RecordingWriter {
        writes: Vec<Vec<u8>>,
        flushes: usize,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes.push(buf.to_vec());
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    fn renderer<'a>(
        out: &'a mut RecordingWriter,
        cfg: RenderConfig,
    ) -> Renderer<'a, RecordingWriter> {
        let plan = plan();
        let board = TtyBoard::new(
            plan.verification,
            plan.criteria,
            cfg.terminal_width,
            cfg.color,
        );
        Renderer {
            out,
            cfg,
            n: plan.criterion_ids.len(),
            check_owner: plan.check_owner,
            checks: plan.checks,
            board,
            started: HashMap::new(),
            running: None,
            line_open: false,
            last_board_lines: Vec::new(),
        }
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
                        expr: None,
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
                        expr: None,
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
                "› AC-1 (1/2)…",
                "✓ AC-1 pass (1.5s)",
                "› AC-2 (2/2)…",
                "✗ AC-2 fail (0.2s)",
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

    /// TTYs redraw one comprehensive criterion → check → step board.
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
                    200,
                    EventPayload::StepFinished {
                        step_index: 0,
                        outcome: duhem_evidence::StepOutcome::Ok,
                    },
                ),
                evt(
                    3,
                    300,
                    EventPayload::AssertionEvaluated {
                        check_id: "AC-1.1".into(),
                        assertion_index: 0,
                        state: VerdictState::Pass,
                        detail: None,
                        expr: None,
                        step_index: Some(0),
                    },
                ),
                evt(
                    4,
                    400,
                    EventPayload::CheckFinished {
                        check_id: "AC-1.1".into(),
                        verdict: VerdictState::Pass,
                    },
                ),
                evt(
                    5,
                    1600,
                    EventPayload::CriterionFinished {
                        criterion_id: "AC-1".into(),
                        verdict: VerdictState::Pass,
                    },
                ),
                evt(
                    6,
                    1650,
                    EventPayload::CheckFinished {
                        check_id: "AC-2.1".into(),
                        verdict: VerdictState::Fail,
                    },
                ),
                evt(
                    7,
                    1700,
                    EventPayload::CriterionFinished {
                        criterion_id: "AC-2".into(),
                        verdict: VerdictState::Fail,
                    },
                ),
                evt(
                    8,
                    1800,
                    EventPayload::RunFinished {
                        verdict: VerdictState::Fail,
                    },
                ),
            ],
            tty_cfg(),
        )
        .await;
        assert!(
            SPINNER
                .iter()
                .any(|frame| out.contains(&format!("{frame} AC-1  one"))),
            "{out:?}"
        );
        assert!(
            SPINNER
                .iter()
                .any(|frame| out.contains(&format!("  └─ {frame} AC-1.1  one check"))),
            "{out:?}"
        );
        assert!(
            SPINNER
                .iter()
                .any(|frame| out.contains(&format!("     └─ {frame} 1/2 cli/invoke"))),
            "{out:?}"
        );
        assert!(out.contains("     └─ ✓ 1/2 cli/invoke"), "{out:?}");
        assert!(out.contains("  └─ ✓ AC-1.1  one check"), "{out:?}");
        assert!(out.contains("✓ AC-1  one"), "{out:?}");
        assert!(out.contains("✗ AC-2  two"), "{out:?}");
        assert!(out.contains("✓ 1 passed   ✗ 1 failed"), "{out:?}");
    }

    #[tokio::test]
    async fn tty_line_shows_check_step_expectation_and_timeout_budget() {
        let mut with = std::collections::BTreeMap::new();
        with.insert("expected".into(), serde_json::json!("visible"));
        with.insert("within".into(), serde_json::json!("60s"));
        let out = rendered(
            vec![
                evt(0, 0, duhem_evidence::run_started("t", Default::default())),
                evt(
                    1,
                    10,
                    EventPayload::StepStarted {
                        criterion_id: "AC-1".into(),
                        check_id: "AC-1.1".into(),
                        step_index: 0,
                        uses: "ui/assert-element".into(),
                        layer: Some("ui".into()),
                        with,
                    },
                ),
            ],
            tty_cfg(),
        )
        .await;
        assert!(
            SPINNER
                .iter()
                .any(|frame| out.contains(&format!("  └─ {frame} AC-1.1  one check"))),
            "{out:?}"
        );
        assert!(
            SPINNER.iter().any(|frame| out.contains(&format!(
                "     └─ {frame} 1/2 ui/assert-element · expect visible"
            ))),
            "{out:?}"
        );
        assert!(out.contains("0s/60s"), "{out:?}");
        assert!(out.contains("────────────────"), "{out:?}");
        assert!(out.contains("1 running   ○ 1 pending"), "{out:?}");
    }

    #[test]
    fn spinner_and_fractional_progress_animate_smoothly() {
        assert_eq!(spinner(Duration::ZERO), "⠋");
        assert_eq!(spinner(Duration::from_millis(250)), "⠙");
        assert_eq!(
            progress_bar(Duration::from_secs(20), Duration::from_secs(60)),
            "█████▎──────────"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tty_board_refresh_is_atomic_and_unchanged_frames_are_skipped() {
        let mut out = RecordingWriter::default();
        let mut renderer = renderer(&mut out, tty_cfg());
        let redraw = evt(
            1,
            0,
            EventPayload::StepStarted {
                criterion_id: "AC-1".into(),
                check_id: "AC-1.1".into(),
                step_index: 0,
                uses: "cli/invoke".into(),
                layer: None,
                with: Default::default(),
            },
        );

        assert!(!renderer.event(redraw));
        assert_eq!(renderer.out.writes.len(), 1);
        assert_eq!(renderer.out.flushes, 1);
        let frame = String::from_utf8_lossy(&renderer.out.writes[0]);
        assert!(frame.starts_with("\x1b[?25l\r\x1b[2K"), "{frame:?}");
        assert!(frame.matches("\r\x1b[2K").count() > 3, "{frame:?}");

        renderer.draw_board();
        assert_eq!(renderer.out.writes.len(), 1);
        assert_eq!(renderer.out.flushes, 1);
    }

    #[test]
    fn status_indicators_are_single_cell_and_rows_stay_aligned() {
        let indicators = [
            INDICATOR_PASS,
            INDICATOR_FAIL,
            INDICATOR_INCONCLUSIVE,
            INDICATOR_PENDING,
            INDICATOR_ACTIVE,
        ];
        for indicator in indicators.into_iter().chain(SPINNER) {
            assert_eq!(
                display_width(indicator),
                1,
                "{indicator:?} must occupy one terminal cell"
            );
            assert!(
                !indicator.contains('\u{fe0f}'),
                "{indicator:?} must not request emoji presentation"
            );
        }

        let widths: Vec<_> = indicators
            .into_iter()
            .chain(SPINNER)
            .map(|indicator| display_width(&format!("{indicator} AC-1  description")))
            .collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] == pair[1]),
            "status rows must align: {widths:?}"
        );
    }

    #[test]
    fn semantic_color_is_ansi_and_plain_width_safe() {
        let plain = "└─ ⠹ running  █▌──  ✓ pass  ✗ fail  ○ pending";
        let colored = colorize(plain);
        assert!(colored.contains("\x1b[36m⠹\x1b[0m"));
        assert!(colored.contains("\x1b[32m✓\x1b[0m"));
        assert!(colored.contains("\x1b[31m✗\x1b[0m"));
        assert!(colored.contains("\x1b[2m○\x1b[0m"));
        let mut stripped = colored;
        for escape in [
            "\x1b[0m",
            "\x1b[36m",
            "\x1b[32m",
            "\x1b[31m",
            "\x1b[33m",
            "\x1b[2m",
            "\x1b[1;36m",
        ] {
            stripped = stripped.replace(escape, "");
        }
        assert_eq!(stripped, plain);
    }

    #[test]
    fn check_summary_truncation_is_utf8_safe() {
        assert_eq!(shorten("short", 5), "short");
        assert_eq!(shorten("回答问题", 2), "回答…");
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
            ..RenderConfig::default()
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
        assert!(out.contains("✓ AC-1 pass"), "{out}");
    }
}
