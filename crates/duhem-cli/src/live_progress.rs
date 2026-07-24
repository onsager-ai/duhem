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
//! - **TTY** — a bounded Ratatui inline viewport uses cell-level
//!   diffs. It keeps the active criterion/check/step expanded,
//!   collapses completed passes to one row, pins failure detail, and
//!   rolls older successful history into an aggregate. The active
//!   leaf has a spinner and fractional timeout bar
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
use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::{Terminal, TerminalOptions, Viewport};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

mod tty;

use tty::TtyBoard;

const TTY_VIEWPORT_HEIGHT: u16 = 12;

/// What the renderer needs to know about the run up front: the
/// ordered criterion ids (for "k/n") and each check's owning
/// criterion (several event kinds carry only a `check_id`).
pub struct Plan {
    verification: String,
    criterion_ids: Vec<String>,
    check_owner: HashMap<String, String>,
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
    /// Bounded cell-diff rendering. Only a real terminal gets it:
    /// when `--live` forces rendering into a capture, control sequences
    /// would garble the log, so `false` keeps plain append-only lines.
    pub tty: bool,
    /// Semantic color. Kept separate from `tty` so the presentation
    /// policy remains independently testable.
    pub color: bool,
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
            ..Self::default()
        }
    }
}

/// Drain the progress channel to stderr until `run_finished` (or the
/// channel closes). Spawned alongside the engine's run future.
pub async fn render_to_stderr(rx: UnboundedReceiver<Event>, plan: Plan, cfg: RenderConfig) {
    if cfg.tty {
        let backend = CrosstermBackend::new(std::io::stderr());
        let terminal = TrackedBackend::at_terminal_bottom(backend).and_then(|backend| {
            Terminal::with_options(
                backend,
                TerminalOptions {
                    viewport: Viewport::Inline(TTY_VIEWPORT_HEIGHT),
                },
            )
        });
        match terminal {
            Ok(terminal) => {
                TtyRenderer::new(terminal, plan, cfg.color).run(rx).await;
            }
            Err(_) => {
                // Live progress is advisory. If the terminal cannot
                // establish an inline viewport, degrade to the same
                // append-only stream used by forced live output.
                let mut err = std::io::stderr();
                render(rx, plan, RenderConfig { tty: false, ..cfg }, &mut err).await;
            }
        }
    } else {
        let mut err = std::io::stderr();
        render(rx, plan, cfg, &mut err).await;
    }
}

/// Append-only fold for non-TTY output. Write errors are ignored —
/// progress is advisory and must never disturb the run.
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
}

/// Crossterm's cursor-position query is hard-wired to stdout on Unix,
/// which would corrupt Duhem's reporter stream even when the backend
/// itself writes to stderr. Track the cursor from the commands Ratatui
/// issues instead. The terminal is first moved to the bottom row; the
/// inline viewport can then reserve its bounded region without a DSR
/// query or any stdout bytes.
struct TrackedBackend<B: Backend> {
    inner: B,
    position: Position,
}

impl<B: Backend> TrackedBackend<B> {
    fn at_terminal_bottom(mut inner: B) -> Result<Self, B::Error> {
        let size = inner.size()?;
        let position = Position::new(0, size.height.saturating_sub(1));
        inner.set_cursor_position(position)?;
        Ok(Self { inner, position })
    }
}

impl<B: Backend> Backend for TrackedBackend<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let content = content.collect::<Vec<_>>();
        let last = content.last().map(|(x, y, cell)| {
            let width = UnicodeWidthStr::width(cell.symbol()).max(1) as u16;
            Position::new(x.saturating_add(width), *y)
        });
        self.inner.draw(content.into_iter())?;
        if let Some(position) = last {
            self.position = position;
        }
        Ok(())
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        self.inner.append_lines(n)?;
        let size = self.inner.size()?;
        self.position.x = 0;
        self.position.y = self
            .position
            .y
            .saturating_add(n)
            .min(size.height.saturating_sub(1));
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        Ok(self.position)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        let position = position.into();
        self.inner.set_cursor_position(position)?;
        self.position = position;
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

/// Interactive terminal fold. Ratatui owns viewport placement,
/// resize bookkeeping, double buffering, and cell-level diffs; this
/// type owns only Duhem's event-to-view-state projection.
struct TtyRenderer<B: Backend> {
    terminal: Terminal<B>,
    board: TtyBoard,
    color: bool,
}

impl<B: Backend> TtyRenderer<B> {
    fn new(terminal: Terminal<B>, plan: Plan, color: bool) -> Self {
        Self {
            terminal,
            board: TtyBoard::new(plan.verification, plan.criteria),
            color,
        }
    }

    async fn run(mut self, mut rx: UnboundedReceiver<Event>) {
        let _ = self.terminal.hide_cursor();
        self.draw();
        let mut tick = tokio::time::interval(TICK_PERIOD);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                evt = rx.recv() => match evt {
                    None => break,
                    Some(evt) => {
                        if self.event(evt) {
                            break;
                        }
                    }
                },
                _ = tick.tick() => {
                    if self.board.is_active() {
                        self.draw();
                    }
                },
            }
        }
        self.close();
    }

    /// Fold one event; `true` means the run is over.
    fn event(&mut self, evt: Event) -> bool {
        if let Some(text) = env_line(&evt.payload) {
            self.insert_line(&text);
            return false;
        }
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
                detail,
                ..
            } => {
                self.board
                    .assertion(check_id, *step_index, state, detail.as_deref());
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
            self.draw();
        }
        done
    }

    fn draw(&mut self) {
        let board = &self.board;
        let color = self.color;
        let _ = self.terminal.draw(|frame| {
            let area = frame.area();
            let lines = board
                .lines(area.width as usize, area.height as usize)
                .into_iter()
                .map(|line| styled_line(&line, color))
                .collect::<Vec<_>>();
            frame.render_widget(Paragraph::new(Text::from(lines)), area);
        });
    }

    fn insert_line(&mut self, text: &str) {
        let line = styled_line(text, self.color);
        let _ = self.terminal.insert_before(1, |buffer| {
            buffer.set_line(buffer.area.x, buffer.area.y, &line, buffer.area.width);
        });
    }

    fn close(&mut self) {
        self.draw();
        let area = self.terminal.get_frame().area();
        let bottom = area.bottom().saturating_sub(1);
        let _ = self
            .terminal
            .set_cursor_position(Position::new(area.x, bottom));
        let _ = self.terminal.show_cursor();
        let _ = self.terminal.backend_mut().append_lines(1);
        let _ = self.terminal.backend_mut().flush();
    }
}

/// The criterion currently narrating, with the monotonic clocks that
/// drive its elapsed display and heartbeat cadence. (Verdict
/// durations use evidence `ts` instead — they must match the stored
/// trace, not this process's clocks.)
struct Running {
    criterion_id: String,
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
}

impl<W: Write> Renderer<'_, W> {
    /// Fold one event; `true` means the run is over.
    fn event(&mut self, evt: Event) -> bool {
        let ts = evt.ts;
        if let Some(text) = env_line(&evt.payload) {
            let _ = writeln!(self.out, "{text}");
            let _ = self.out.flush();
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
            }
            EventPayload::StepFinished { .. } => {
                if let Some(run) = &mut self.running {
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
            EventPayload::CheckFinished { check_id, .. } => {
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
                let mark = verdict_mark(verdict);
                let secs = self
                    .started
                    .get(criterion_id)
                    .map(|(_, t0)| (ts - *t0).num_milliseconds() as f64 / 1000.0)
                    .unwrap_or(0.0);
                // One final durable line per criterion (#305 E).
                let _ = writeln!(self.out, "{mark} {criterion_id} {verdict} ({secs:.1}s)");
                self.running = None;
            }
            EventPayload::RunFinished { .. } => return true,
            _ => {}
        }
        false
    }

    /// Periodic wake with no event: consider an append heartbeat.
    fn tick(&mut self) {
        if self.running.is_none() {
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
            step_uses: None,
            step_since: Instant::now(),
            beats: 0,
        });
        let n = self.n;
        let _ = writeln!(self.out, "{INDICATOR_ACTIVE} {criterion_id} ({k}/{n})…");
    }
}

fn verdict_mark(verdict: &duhem_judge::VerdictState) -> &'static str {
    match verdict {
        duhem_judge::VerdictState::Pass => INDICATOR_PASS,
        duhem_judge::VerdictState::Fail => INDICATOR_FAIL,
        duhem_judge::VerdictState::Inconclusive(_) => INDICATOR_INCONCLUSIVE,
    }
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

fn styled_line(text: &str, color: bool) -> Line<'static> {
    if !color {
        return Line::raw(text.to_string());
    }
    if text.contains(" Duhem  ") {
        return Line::styled(
            text.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    }

    let mut spans = Vec::new();
    let mut current = Style::default();
    let mut content = String::new();
    for ch in text.chars() {
        let style = match ch {
            '✓' => Style::default().fg(Color::Green),
            '✗' => Style::default().fg(Color::Red),
            '◐' => Style::default().fg(Color::Yellow),
            '○' | '│' | '├' | '└' | '─' | '…' => {
                Style::default().add_modifier(Modifier::DIM)
            }
            '█' | '▏' | '▎' | '▍' | '▌' | '▋' | '▊' | '▉' | '›' => {
                Style::default().fg(Color::Cyan)
            }
            ch if SPINNER.iter().any(|frame| frame.starts_with(ch)) => {
                Style::default().fg(Color::Cyan)
            }
            _ => Style::default(),
        };
        if style != current && !content.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut content), current));
        }
        current = style;
        content.push(ch);
    }
    if !content.is_empty() {
        spans.push(Span::styled(content, current));
    }
    Line::from(spans)
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
    use std::convert::Infallible;

    use super::*;
    use duhem_evidence::EventPayload;
    use duhem_judge::VerdictState;
    use ratatui::backend::TestBackend;

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

    struct CountingBackend {
        inner: TestBackend,
        drawn_cells: usize,
    }

    impl CountingBackend {
        fn new(width: u16, height: u16) -> Self {
            Self {
                inner: TestBackend::new(width, height),
                drawn_cells: 0,
            }
        }
    }

    impl Backend for CountingBackend {
        type Error = Infallible;

        fn draw<'a, I>(&mut self, content: I) -> Result<(), Infallible>
        where
            I: Iterator<Item = (u16, u16, &'a Cell)>,
        {
            let content = content.collect::<Vec<_>>();
            self.drawn_cells += content.len();
            self.inner.draw(content.into_iter())
        }

        fn append_lines(&mut self, n: u16) -> Result<(), Infallible> {
            self.inner.append_lines(n)
        }

        fn hide_cursor(&mut self) -> Result<(), Infallible> {
            self.inner.hide_cursor()
        }

        fn show_cursor(&mut self) -> Result<(), Infallible> {
            self.inner.show_cursor()
        }

        fn get_cursor_position(&mut self) -> Result<Position, Infallible> {
            self.inner.get_cursor_position()
        }

        fn set_cursor_position<P: Into<Position>>(
            &mut self,
            position: P,
        ) -> Result<(), Infallible> {
            self.inner.set_cursor_position(position)
        }

        fn clear(&mut self) -> Result<(), Infallible> {
            self.inner.clear()
        }

        fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Infallible> {
            self.inner.clear_region(clear_type)
        }

        fn size(&self) -> Result<Size, Infallible> {
            self.inner.size()
        }

        fn window_size(&mut self) -> Result<WindowSize, Infallible> {
            self.inner.window_size()
        }

        fn flush(&mut self) -> Result<(), Infallible> {
            self.inner.flush()
        }
    }

    fn tty_renderer(width: u16, height: u16) -> TtyRenderer<CountingBackend> {
        let terminal = Terminal::with_options(
            CountingBackend::new(width, height),
            TerminalOptions {
                viewport: Viewport::Inline(height),
            },
        )
        .unwrap();
        TtyRenderer::new(terminal, plan(), false)
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

    #[tokio::test(start_paused = true)]
    async fn tty_expands_active_work_and_collapses_completed_passes() {
        let mut renderer = tty_renderer(120, 12);
        assert!(!renderer.event(evt(
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
        )));
        let active = renderer.board.lines(120, 12).join("\n");
        assert!(
            SPINNER
                .iter()
                .any(|frame| active.contains(&format!("{frame} AC-1  one"))),
            "{active:?}"
        );
        assert!(
            SPINNER
                .iter()
                .any(|frame| active.contains(&format!("  └─ {frame} AC-1.1  one check"))),
            "{active:?}"
        );
        assert!(
            SPINNER
                .iter()
                .any(|frame| active.contains(&format!("     └─ {frame} 1/2 cli/invoke"))),
            "{active:?}"
        );

        for event in [
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
        ] {
            assert!(!renderer.event(event));
        }
        let completed = renderer.board.lines(120, 12).join("\n");
        assert!(completed.contains("✓ AC-1  one  1/1"), "{completed:?}");
        assert!(
            !completed.contains("1/2 cli/invoke"),
            "passing step history must collapse: {completed:?}"
        );
        assert!(completed.contains("✗ AC-2  two  1/1"), "{completed:?}");
        assert!(completed.contains("└─ ✗ AC-2.1"), "{completed:?}");
        assert!(
            completed.contains("✓ 1 passed   ✗ 1 failed"),
            "{completed:?}"
        );
    }

    fn rolling_board() -> TtyBoard {
        let criteria = (1..=8)
            .map(|index| CriterionPlan {
                id: format!("AC-{index}"),
                description: format!("criterion {index}"),
                checks: vec![CheckPlan {
                    id: format!("AC-{index}.1"),
                    description: Some(format!("check {index}")),
                    step_count: 1,
                }],
            })
            .collect();
        let mut board = TtyBoard::new("rolling".into(), criteria);
        let with = std::collections::BTreeMap::new();
        for index in 1..=5 {
            let criterion = format!("AC-{index}");
            let check = format!("{criterion}.1");
            board.start_step(&criterion, &check, 0, "cli/invoke", &with);
            board.finish_step(&duhem_evidence::StepOutcome::Ok);
            board.assertion(&check, Some(0), &VerdictState::Pass, None);
            board.finish_check(&check, &VerdictState::Pass);
            board.finish_criterion(&criterion, &VerdictState::Pass);
        }
        board.start_step("AC-6", "AC-6.1", 0, "api/call", &with);
        board.finish_step(&duhem_evidence::StepOutcome::Error);
        board.assertion(
            "AC-6.1",
            Some(0),
            &VerdictState::Fail,
            Some("expected HTTP 402, received 500"),
        );
        board.finish_check("AC-6.1", &VerdictState::Fail);
        board.finish_criterion("AC-6", &VerdictState::Fail);
        board.start_step("AC-7", "AC-7.1", 0, "ui/assert-element", &with);
        board
    }

    #[test]
    fn rolling_projection_bounds_history_and_prioritizes_failures_and_active_work() {
        let board = rolling_board();
        let tall = board.lines(100, 12);
        let tall_text = tall.join("\n");
        assert_eq!(tall.len(), 12);
        assert!(
            tall_text.contains("earlier passing criteria collapsed"),
            "{tall_text}"
        );
        assert!(tall_text.contains("✗ AC-6"), "{tall_text}");
        assert!(
            tall_text.contains("expected HTTP 402, received 500"),
            "{tall_text}"
        );
        assert!(tall_text.contains("AC-7"), "{tall_text}");
        assert!(tall_text.contains("ui/assert-element"), "{tall_text}");
        assert!(
            !tall_text.contains("AC-1.1"),
            "completed passing checks must collapse: {tall_text}"
        );

        let short = board.lines(70, 7);
        let short_text = short.join("\n");
        assert_eq!(short.len(), 7);
        assert!(short_text.contains("✗ AC-6"), "{short_text}");
        assert!(short_text.contains("AC-7"), "{short_text}");
        assert!(short_text.contains("ui/assert-element"), "{short_text}");
    }

    #[tokio::test]
    async fn tty_line_shows_check_step_expectation_and_timeout_budget() {
        let mut with = std::collections::BTreeMap::new();
        with.insert("expected".into(), serde_json::json!("visible"));
        with.insert("within".into(), serde_json::json!("60s"));
        let mut renderer = tty_renderer(120, 12);
        assert!(!renderer.event(evt(
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
        )));
        let out = renderer.board.lines(120, 12).join("\n");
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
    async fn tty_animation_tick_writes_only_changed_cells() {
        let mut renderer = tty_renderer(120, 12);
        assert!(!renderer.event(evt(
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
        )));
        renderer.terminal.backend_mut().drawn_cells = 0;

        renderer.draw();
        assert_eq!(
            renderer.terminal.backend().drawn_cells,
            0,
            "an unchanged frame must produce no backend cell writes"
        );

        tokio::time::advance(TICK_PERIOD).await;
        renderer.draw();
        let changed = renderer.terminal.backend().drawn_cells;
        assert!(
            (1..=8).contains(&changed),
            "a spinner-only tick should touch a handful of cells, got {changed}"
        );
    }

    #[test]
    fn tty_viewport_resizes_and_restores_the_cursor() {
        let mut renderer = tty_renderer(80, 12);
        renderer.draw();
        renderer.terminal.backend_mut().inner.resize(70, 6);
        renderer.draw();
        assert_eq!(renderer.terminal.get_frame().area().height, 6);
        assert!(renderer.board.lines(70, 6).len() <= 6);

        let _ = renderer.terminal.hide_cursor();
        renderer.close();
        assert!(renderer.terminal.backend().inner.cursor_visible());
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
    fn semantic_color_is_structured_and_plain_width_safe() {
        let plain = "└─ ⠹ running  █▌──  ✓ pass  ✗ fail  ○ pending";
        let colored = styled_line(plain, true);
        assert_eq!(colored.to_string(), plain);
        let style_for = |needle: char| {
            colored
                .spans
                .iter()
                .find(|span| span.content.contains(needle))
                .map(|span| span.style)
                .unwrap()
        };
        assert_eq!(style_for('⠹').fg, Some(Color::Cyan));
        assert_eq!(style_for('✓').fg, Some(Color::Green));
        assert_eq!(style_for('✗').fg, Some(Color::Red));
        assert!(style_for('○').add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn check_summary_truncation_is_utf8_safe() {
        assert_eq!(shorten_to_width("short", 5), "short");
        assert_eq!(shorten_to_width("回答问题", 5), "回答…");
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
