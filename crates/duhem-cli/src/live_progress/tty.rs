//! Stateful criterion → check → step board for an interactive terminal.

use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;

use super::{
    CriterionPlan, INDICATOR_FAIL, INDICATOR_INCONCLUSIVE, INDICATOR_PASS, INDICATOR_PENDING,
    expectation, progress_bar, shorten_to_width, spinner, timeout, verdict_mark,
};

pub(super) struct TtyBoard {
    verification: String,
    criteria: Vec<CriterionPlan>,
    states: HashMap<String, CriterionState>,
    active_criterion: Option<String>,
    active_check: Option<String>,
    active_step: Option<ActiveStep>,
    since: Instant,
}

#[derive(Default)]
struct CriterionState {
    verdict: Option<duhem_judge::VerdictState>,
    checks: HashMap<String, CheckState>,
    started: Option<Instant>,
    duration: Option<Duration>,
}

#[derive(Default)]
struct CheckState {
    verdict: Option<duhem_judge::VerdictState>,
    steps: Vec<CompletedStep>,
    started: Option<Instant>,
    duration: Option<Duration>,
    detail: Option<String>,
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
    detail: Option<String>,
    duration: Duration,
}

impl TtyBoard {
    pub(super) fn new(verification: String, criteria: Vec<CriterionPlan>) -> Self {
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
        let now = Instant::now();
        self.active_criterion = Some(criterion_id.to_string());
        self.active_check = Some(check_id.to_string());
        self.active_step = Some(ActiveStep {
            index,
            uses: uses.to_string(),
            expectation: expectation(with),
            timeout: timeout(with),
            since: now,
        });
        let criterion = self.states.entry(criterion_id.to_string()).or_default();
        criterion.started.get_or_insert(now);
        criterion
            .checks
            .entry(check_id.to_string())
            .or_default()
            .started
            .get_or_insert(now);
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
                detail: None,
                duration: step.since.elapsed(),
            });
    }

    pub(super) fn assertion(
        &mut self,
        check_id: &str,
        step_index: Option<u32>,
        state: &duhem_judge::VerdictState,
        detail: Option<&str>,
    ) {
        let Some(criterion_id) = self.owner(check_id).map(str::to_string) else {
            return;
        };
        let now = Instant::now();
        self.active_criterion = Some(criterion_id.clone());
        self.active_check = Some(check_id.to_string());
        let criterion = self.states.entry(criterion_id).or_default();
        criterion.started.get_or_insert(now);
        let check = criterion.checks.entry(check_id.to_string()).or_default();
        check.started.get_or_insert(now);
        check.detail = detail.map(str::to_string);
        if let Some(index) = step_index
            && let Some(step) = check.steps.iter_mut().find(|step| step.index == index)
        {
            step.judgment = Some(*state);
            step.detail = detail.map(str::to_string);
        }
    }

    pub(super) fn finish_check(&mut self, check_id: &str, verdict: &duhem_judge::VerdictState) {
        let Some(criterion_id) = self.owner(check_id).map(str::to_string) else {
            return;
        };
        let now = Instant::now();
        let criterion = self.states.entry(criterion_id.clone()).or_default();
        criterion.started.get_or_insert(now);
        let check = criterion.checks.entry(check_id.to_string()).or_default();
        check.started.get_or_insert(now);
        check.duration = check.started.map(|started| started.elapsed());
        check.verdict = Some(*verdict);
        self.active_criterion = Some(criterion_id);
        self.active_check = None;
        self.active_step = None;
    }

    pub(super) fn finish_criterion(
        &mut self,
        criterion_id: &str,
        verdict: &duhem_judge::VerdictState,
    ) {
        let criterion = self.states.entry(criterion_id.to_string()).or_default();
        criterion.duration = criterion.started.map(|started| started.elapsed());
        criterion.verdict = Some(*verdict);
        self.active_criterion = None;
        self.active_check = None;
        self.active_step = None;
    }

    pub(super) fn is_active(&self) -> bool {
        self.active_step.is_some()
    }

    fn owner(&self, check_id: &str) -> Option<&str> {
        self.criteria
            .iter()
            .find(|criterion| criterion.checks.iter().any(|check| check.id == check_id))
            .map(|criterion| criterion.id.as_str())
    }

    /// Project the full run state into a bounded rolling viewport.
    ///
    /// The active subtree and non-pass details have priority. Passing
    /// criteria are one row each and roll into an aggregate when the
    /// viewport cannot hold them. This is intentionally independent of
    /// the terminal backend so the information policy is mechanically
    /// testable.
    pub(super) fn lines(&self, terminal_width: usize, terminal_height: usize) -> Vec<String> {
        let elapsed = self.since.elapsed().as_secs();
        let activity = spinner(self.since.elapsed());
        let run_mark = self.run_mark(activity);
        let header = fit(
            format!(
                "{run_mark} Duhem  {}  ·  elapsed {:02}:{:02}",
                self.verification,
                elapsed / 60,
                elapsed % 60
            ),
            terminal_width,
        );
        let summary = fit(self.summary(activity), terminal_width);

        if terminal_height <= 1 {
            return vec![header];
        }
        if terminal_height == 2 {
            return vec![header, summary];
        }

        // Header + spacer + body + spacer + summary. Tiny terminals
        // degrade to header/body/summary without the spacers.
        let with_spacers = terminal_height >= 9;
        let chrome = if with_spacers { 4 } else { 2 };
        let body_budget = terminal_height.saturating_sub(chrome);

        let mut passing = Vec::new();
        let mut failures = Vec::new();
        let mut active = Vec::new();
        for criterion in &self.criteria {
            let state = &self.states[&criterion.id];
            let is_active = self.active_criterion.as_deref() == Some(criterion.id.as_str());
            if is_active {
                active = self.active_rows(criterion, state, activity, terminal_width);
            } else {
                match state.verdict.as_ref() {
                    Some(duhem_judge::VerdictState::Pass) => passing.push(fit(
                        self.criterion_row(criterion, state, INDICATOR_PASS),
                        terminal_width,
                    )),
                    Some(duhem_judge::VerdictState::Fail)
                    | Some(duhem_judge::VerdictState::Inconclusive(_)) => {
                        failures.extend(self.failure_rows(criterion, state, terminal_width))
                    }
                    None => {}
                }
            }
        }

        let active = clip_rows(
            active,
            body_budget,
            "… active details collapsed",
            terminal_width,
        );
        let failure_budget = body_budget.saturating_sub(active.len());
        let failures = clip_rows(
            failures,
            failure_budget,
            "… more failure details",
            terminal_width,
        );
        let pass_budget = body_budget.saturating_sub(active.len() + failures.len());
        let passing = rolling_passes(passing, pass_budget, terminal_width);

        let mut lines = Vec::with_capacity(terminal_height);
        lines.push(header);
        if with_spacers {
            lines.push(String::new());
        }
        lines.extend(passing);
        lines.extend(failures);
        lines.extend(active);
        if with_spacers {
            lines.push(String::new());
        }
        lines.push(summary);
        lines.truncate(terminal_height);
        lines
    }

    fn criterion_row(
        &self,
        criterion: &CriterionPlan,
        state: &CriterionState,
        mark: &str,
    ) -> String {
        let completed = criterion
            .checks
            .iter()
            .filter(|check| {
                state
                    .checks
                    .get(&check.id)
                    .is_some_and(|state| state.verdict.is_some())
            })
            .count();
        let progress = if criterion.checks.is_empty() {
            String::new()
        } else {
            format!("  {completed}/{}", criterion.checks.len())
        };
        let duration = duration_suffix(state.duration);
        format!(
            "{mark} {}  {}{progress}{duration}",
            criterion.id,
            single_line(&criterion.description)
        )
    }

    fn active_rows(
        &self,
        criterion: &CriterionPlan,
        state: &CriterionState,
        activity: &str,
        terminal_width: usize,
    ) -> Vec<String> {
        let mut rows = vec![fit(
            self.criterion_row(criterion, state, activity),
            terminal_width,
        )];
        let visible_checks: Vec<_> = criterion
            .checks
            .iter()
            .filter(|check| state.checks.contains_key(&check.id))
            .collect();
        for (position, check) in visible_checks.iter().enumerate() {
            let last = position + 1 == visible_checks.len();
            let branch = if last { "└─" } else { "├─" };
            let check_state = &state.checks[&check.id];
            let is_active = self.active_check.as_deref() == Some(check.id.as_str());
            let mark = check_state
                .verdict
                .as_ref()
                .map(verdict_mark)
                .unwrap_or(if is_active {
                    activity
                } else {
                    INDICATOR_PENDING
                });
            rows.push(fit(
                format!(
                    "  {branch} {mark} {}  {}{}",
                    check.id,
                    check
                        .description
                        .as_deref()
                        .map(single_line)
                        .unwrap_or_default(),
                    duration_suffix(check_state.duration)
                ),
                terminal_width,
            ));
            if !is_active {
                continue;
            }
            let child_prefix = if last { "     " } else { "  │  " };
            if !check_state.steps.is_empty() {
                let branch = if self.active_step.is_some() {
                    "├─"
                } else {
                    "└─"
                };
                let mark = completed_steps_mark(&check_state.steps);
                rows.push(fit(
                    format!(
                        "{child_prefix}{branch} {mark} {} completed step{}",
                        check_state.steps.len(),
                        if check_state.steps.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ),
                    terminal_width,
                ));
            }
            if let Some(step) = &self.active_step {
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
                rows.push(fit(
                    format!(
                        "{child_prefix}└─ {activity} {}/{} {}{expectation}{progress}",
                        step.index + 1,
                        check.step_count,
                        step.uses
                    ),
                    terminal_width,
                ));
            }
        }
        rows
    }

    fn failure_rows(
        &self,
        criterion: &CriterionPlan,
        state: &CriterionState,
        terminal_width: usize,
    ) -> Vec<String> {
        let mark = state
            .verdict
            .as_ref()
            .map(verdict_mark)
            .unwrap_or(INDICATOR_INCONCLUSIVE);
        let mut rows = vec![fit(
            self.criterion_row(criterion, state, mark),
            terminal_width,
        )];
        let failed_checks: Vec<_> = criterion
            .checks
            .iter()
            .filter_map(|check| {
                let state = state.checks.get(&check.id)?;
                (!matches!(state.verdict, Some(duhem_judge::VerdictState::Pass)))
                    .then_some((check, state))
            })
            .collect();
        for (position, (check, check_state)) in failed_checks.iter().enumerate() {
            let branch = if position + 1 == failed_checks.len() {
                "└─"
            } else {
                "├─"
            };
            let mark = check_state
                .verdict
                .as_ref()
                .map(verdict_mark)
                .unwrap_or(INDICATOR_INCONCLUSIVE);
            rows.push(fit(
                format!(
                    "  {branch} {mark} {}  {}{}",
                    check.id,
                    check
                        .description
                        .as_deref()
                        .map(single_line)
                        .unwrap_or_default(),
                    detail_suffix(check_state.detail.as_deref())
                ),
                terminal_width,
            ));
            if let Some(step) = check_state.steps.iter().rev().find(|step| {
                !matches!(step.judgment, Some(duhem_judge::VerdictState::Pass) | None)
                    || !matches!(step.outcome, duhem_evidence::StepOutcome::Ok)
            }) {
                rows.push(fit(
                    format!(
                        "     └─ {} {}/{} {}{}{}",
                        step_mark(step),
                        step.index + 1,
                        check.step_count,
                        step.uses,
                        duration_suffix(Some(step.duration)),
                        detail_suffix(step.detail.as_deref())
                    ),
                    terminal_width,
                ));
            }
        }
        rows
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
            return INDICATOR_FAIL;
        }
        if self.criteria.iter().any(|criterion| {
            matches!(
                self.states[&criterion.id].verdict,
                Some(duhem_judge::VerdictState::Inconclusive(_))
            )
        }) {
            return INDICATOR_INCONCLUSIVE;
        }
        INDICATOR_PASS
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
            parts.push(format!("{INDICATOR_PASS} {passed} passed"));
        }
        if failed > 0 {
            parts.push(format!("{INDICATOR_FAIL} {failed} failed"));
        }
        if inconclusive > 0 {
            parts.push(format!(
                "{INDICATOR_INCONCLUSIVE} {inconclusive} inconclusive"
            ));
        }
        if self.active_criterion.is_some() {
            parts.push(format!("{activity} 1 running"));
        }
        if pending > 0 {
            parts.push(format!("{INDICATOR_PENDING} {pending} pending"));
        }
        parts.join("   ")
    }
}

fn duration_suffix(duration: Option<Duration>) -> String {
    duration
        .map(|duration| format!("  · {:.1}s", duration.as_secs_f64()))
        .unwrap_or_default()
}

fn detail_suffix(detail: Option<&str>) -> String {
    detail
        .filter(|detail| !detail.trim().is_empty())
        .map(|detail| format!("  · {}", single_line(detail)))
        .unwrap_or_default()
}

fn fit(line: String, terminal_width: usize) -> String {
    shorten_to_width(&line, terminal_width)
}

fn clip_rows(
    rows: Vec<String>,
    budget: usize,
    omitted_label: &str,
    terminal_width: usize,
) -> Vec<String> {
    if rows.len() <= budget {
        return rows;
    }
    if budget == 0 {
        return Vec::new();
    }
    if budget == 1 {
        return vec![fit(omitted_label.to_string(), terminal_width)];
    }
    if budget == 2 {
        return vec![
            rows[0].clone(),
            fit(omitted_label.to_string(), terminal_width),
        ];
    }
    let tail = budget - 2;
    let mut clipped = Vec::with_capacity(budget);
    clipped.push(rows[0].clone());
    clipped.push(fit(omitted_label.to_string(), terminal_width));
    clipped.extend(
        rows.into_iter()
            .rev()
            .take(tail)
            .collect::<Vec<_>>()
            .into_iter()
            .rev(),
    );
    clipped
}

fn rolling_passes(rows: Vec<String>, budget: usize, terminal_width: usize) -> Vec<String> {
    if rows.len() <= budget {
        return rows;
    }
    if budget == 0 {
        return Vec::new();
    }
    let visible = budget.saturating_sub(1);
    let hidden = rows.len().saturating_sub(visible);
    let mut projected = vec![fit(
        format!("… {hidden} earlier passing criteria collapsed"),
        terminal_width,
    )];
    projected.extend(
        rows.into_iter()
            .rev()
            .take(visible)
            .collect::<Vec<_>>()
            .into_iter()
            .rev(),
    );
    projected
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn step_mark(step: &CompletedStep) -> &'static str {
    if let Some(judgment) = &step.judgment {
        return verdict_mark(judgment);
    }
    match step.outcome {
        duhem_evidence::StepOutcome::Ok => INDICATOR_PASS,
        duhem_evidence::StepOutcome::Error => INDICATOR_FAIL,
        duhem_evidence::StepOutcome::Timeout => INDICATOR_INCONCLUSIVE,
    }
}

fn completed_steps_mark(steps: &[CompletedStep]) -> &'static str {
    if steps.iter().any(|step| step_mark(step) == INDICATOR_FAIL) {
        return INDICATOR_FAIL;
    }
    if steps
        .iter()
        .any(|step| step_mark(step) == INDICATOR_INCONCLUSIVE)
    {
        return INDICATOR_INCONCLUSIVE;
    }
    INDICATOR_PASS
}
