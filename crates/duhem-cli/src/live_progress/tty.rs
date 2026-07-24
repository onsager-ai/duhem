//! Stateful criterion → check → step board for an interactive terminal.

use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;

use super::{
    CriterionPlan, colorize, expectation, progress_bar, shorten_to_width, spinner, timeout,
    verdict_mark,
};

pub(super) struct TtyBoard {
    verification: String,
    criteria: Vec<CriterionPlan>,
    states: HashMap<String, CriterionState>,
    active_criterion: Option<String>,
    active_check: Option<String>,
    active_step: Option<ActiveStep>,
    terminal_width: usize,
    color: bool,
    frame_height: usize,
    cursor_hidden: bool,
    since: Instant,
}

#[derive(Default)]
struct CriterionState {
    verdict: Option<duhem_judge::VerdictState>,
    checks: HashMap<String, CheckState>,
}

#[derive(Default)]
struct CheckState {
    verdict: Option<duhem_judge::VerdictState>,
    steps: Vec<CompletedStep>,
}

struct ActiveStep {
    index: u32,
    uses: String,
    expectation: Option<String>,
    timeout: Option<Duration>,
    since: Instant,
}

struct CompletedStep {
    index: u32,
    uses: String,
    outcome: duhem_evidence::StepOutcome,
    judgment: Option<duhem_judge::VerdictState>,
    duration: Duration,
}

impl TtyBoard {
    pub(super) fn new(
        verification: String,
        criteria: Vec<CriterionPlan>,
        terminal_width: usize,
        color: bool,
    ) -> Self {
        let states = criteria
            .iter()
            .map(|criterion| (criterion.id.clone(), CriterionState::default()))
            .collect();
        Self {
            verification,
            criteria,
            states,
            active_criterion: None,
            active_check: None,
            active_step: None,
            terminal_width,
            color,
            frame_height: 0,
            cursor_hidden: false,
            since: Instant::now(),
        }
    }

    pub(super) fn start_step(
        &mut self,
        criterion_id: &str,
        check_id: &str,
        index: u32,
        uses: &str,
        with: &std::collections::BTreeMap<String, serde_json::Value>,
    ) {
        self.active_criterion = Some(criterion_id.to_string());
        self.active_check = Some(check_id.to_string());
        self.active_step = Some(ActiveStep {
            index,
            uses: uses.to_string(),
            expectation: expectation(with),
            timeout: timeout(with),
            since: Instant::now(),
        });
        self.states
            .entry(criterion_id.to_string())
            .or_default()
            .checks
            .entry(check_id.to_string())
            .or_default();
    }

    pub(super) fn finish_step(&mut self, outcome: &duhem_evidence::StepOutcome) {
        let (Some(criterion_id), Some(check_id), Some(step)) = (
            self.active_criterion.as_deref(),
            self.active_check.as_deref(),
            self.active_step.take(),
        ) else {
            return;
        };
        self.states
            .entry(criterion_id.to_string())
            .or_default()
            .checks
            .entry(check_id.to_string())
            .or_default()
            .steps
            .push(CompletedStep {
                index: step.index,
                uses: step.uses,
                outcome: outcome.clone(),
                judgment: None,
                duration: step.since.elapsed(),
            });
    }

    pub(super) fn assertion(
        &mut self,
        check_id: &str,
        step_index: Option<u32>,
        state: &duhem_judge::VerdictState,
    ) {
        let Some(criterion_id) = self.owner(check_id).map(str::to_string) else {
            return;
        };
        self.active_criterion = Some(criterion_id.clone());
        self.active_check = Some(check_id.to_string());
        if let Some(index) = step_index
            && let Some(step) = self
                .states
                .get_mut(&criterion_id)
                .and_then(|criterion| criterion.checks.get_mut(check_id))
                .and_then(|check| check.steps.iter_mut().find(|step| step.index == index))
        {
            step.judgment = Some(*state);
        }
    }

    pub(super) fn finish_check(&mut self, check_id: &str, verdict: &duhem_judge::VerdictState) {
        let Some(criterion_id) = self.owner(check_id).map(str::to_string) else {
            return;
        };
        self.states
            .entry(criterion_id.clone())
            .or_default()
            .checks
            .entry(check_id.to_string())
            .or_default()
            .verdict = Some(*verdict);
        self.active_criterion = Some(criterion_id);
        self.active_check = None;
        self.active_step = None;
    }

    pub(super) fn finish_criterion(
        &mut self,
        criterion_id: &str,
        verdict: &duhem_judge::VerdictState,
    ) {
        self.states
            .entry(criterion_id.to_string())
            .or_default()
            .verdict = Some(*verdict);
        self.active_criterion = None;
        self.active_check = None;
        self.active_step = None;
    }

    pub(super) fn is_active(&self) -> bool {
        self.active_step.is_some()
    }

    pub(super) fn frame_height(&self) -> usize {
        self.frame_height
    }

    pub(super) fn set_frame_height(&mut self, height: usize) {
        self.frame_height = height;
    }

    pub(super) fn cursor_hidden(&self) -> bool {
        self.cursor_hidden
    }

    pub(super) fn set_cursor_hidden(&mut self, hidden: bool) {
        self.cursor_hidden = hidden;
    }

    fn owner(&self, check_id: &str) -> Option<&str> {
        self.criteria
            .iter()
            .find(|criterion| criterion.checks.iter().any(|check| check.id == check_id))
            .map(|criterion| criterion.id.as_str())
    }

    pub(super) fn lines(&self) -> Vec<String> {
        let elapsed = self.since.elapsed().as_secs();
        let activity = spinner(self.since.elapsed());
        let run_mark = self.run_mark(activity);
        let mut lines = vec![
            self.fit(format!(
                "{run_mark} Duhem  {}  ·  elapsed {:02}:{:02}",
                self.verification,
                elapsed / 60,
                elapsed % 60
            )),
            String::new(),
        ];
        for criterion in &self.criteria {
            let state = &self.states[&criterion.id];
            let active = self.active_criterion.as_deref() == Some(criterion.id.as_str());
            let mark = state
                .verdict
                .as_ref()
                .map(verdict_mark)
                .unwrap_or(if active { activity } else { "○" });
            lines.push(self.fit(format!(
                "{mark} {}  {}",
                criterion.id,
                single_line(&criterion.description)
            )));

            let started = state.verdict.is_some()
                || active
                || criterion
                    .checks
                    .iter()
                    .any(|check| state.checks.contains_key(&check.id));
            if !started {
                continue;
            }

            for (check_pos, check) in criterion.checks.iter().enumerate() {
                let last_check = check_pos + 1 == criterion.checks.len();
                let check_state = state.checks.get(&check.id);
                let check_active =
                    active && self.active_check.as_deref() == Some(check.id.as_str());
                let check_mark = check_state
                    .and_then(|state| state.verdict.as_ref())
                    .map(verdict_mark)
                    .unwrap_or(if check_active { activity } else { "○" });
                let check_branch = if last_check { "└─" } else { "├─" };
                let summary = check
                    .description
                    .as_deref()
                    .map(single_line)
                    .unwrap_or_default();
                lines.push(self.fit(format!(
                    "  {check_branch} {check_mark} {}  {summary}",
                    check.id
                )));

                let Some(check_state) = check_state else {
                    continue;
                };
                let child_prefix = if last_check { "     " } else { "  │  " };
                let has_active_step = check_active && self.active_step.is_some();
                for (step_pos, step) in check_state.steps.iter().enumerate() {
                    let mark = step_mark(step);
                    let last_step = step_pos + 1 == check_state.steps.len() && !has_active_step;
                    let branch = if last_step { "└─" } else { "├─" };
                    lines.push(self.fit(format!(
                        "{child_prefix}{branch} {mark} {}/{} {}  {:.1}s",
                        step.index + 1,
                        check.step_count,
                        step.uses,
                        step.duration.as_secs_f64()
                    )));
                }
                if check_active && let Some(step) = &self.active_step {
                    let expectation = step
                        .expectation
                        .as_deref()
                        .map(|value| format!(" · {value}"))
                        .unwrap_or_default();
                    let progress = match step.timeout {
                        Some(timeout) => format!(
                            "  {}  {}s/{}s",
                            progress_bar(step.since.elapsed(), timeout),
                            step.since.elapsed().as_secs(),
                            timeout.as_secs()
                        ),
                        None => format!("  {}s", step.since.elapsed().as_secs()),
                    };
                    lines.push(self.fit(format!(
                        "{child_prefix}└─ {activity} {}/{} {}{expectation}{progress}",
                        step.index + 1,
                        check.step_count,
                        step.uses
                    )));
                }
            }
        }
        lines.push(String::new());
        lines.push(self.fit(self.summary(activity)));
        lines
    }

    fn run_mark<'a>(&self, activity: &'a str) -> &'a str {
        if self
            .criteria
            .iter()
            .any(|criterion| self.states[&criterion.id].verdict.is_none())
        {
            return activity;
        }
        if self.criteria.iter().any(|criterion| {
            matches!(
                self.states[&criterion.id].verdict,
                Some(duhem_judge::VerdictState::Fail)
            )
        }) {
            return "✘";
        }
        if self.criteria.iter().any(|criterion| {
            matches!(
                self.states[&criterion.id].verdict,
                Some(duhem_judge::VerdictState::Inconclusive(_))
            )
        }) {
            return "◐";
        }
        "✔"
    }

    fn summary(&self, activity: &str) -> String {
        let mut passed = 0;
        let mut failed = 0;
        let mut inconclusive = 0;
        let mut pending = 0;
        for criterion in &self.criteria {
            match self.states[&criterion.id].verdict {
                Some(duhem_judge::VerdictState::Pass) => passed += 1,
                Some(duhem_judge::VerdictState::Fail) => failed += 1,
                Some(duhem_judge::VerdictState::Inconclusive(_)) => inconclusive += 1,
                None if self.active_criterion.as_deref() != Some(criterion.id.as_str()) => {
                    pending += 1;
                }
                None => {}
            }
        }
        let mut parts = Vec::new();
        if passed > 0 {
            parts.push(format!("✔ {passed} passed"));
        }
        if failed > 0 {
            parts.push(format!("✘ {failed} failed"));
        }
        if inconclusive > 0 {
            parts.push(format!("◐ {inconclusive} inconclusive"));
        }
        if self.active_criterion.is_some() {
            parts.push(format!("{activity} 1 running"));
        }
        if pending > 0 {
            parts.push(format!("○ {pending} pending"));
        }
        parts.join("   ")
    }

    fn fit(&self, line: String) -> String {
        let plain = shorten_to_width(&line, self.terminal_width);
        if self.color { colorize(&plain) } else { plain }
    }
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn step_mark(step: &CompletedStep) -> &'static str {
    if let Some(judgment) = &step.judgment {
        return verdict_mark(judgment);
    }
    match step.outcome {
        duhem_evidence::StepOutcome::Ok => "✓",
        duhem_evidence::StepOutcome::Error => "✘",
        duhem_evidence::StepOutcome::Timeout => "◐",
    }
}
