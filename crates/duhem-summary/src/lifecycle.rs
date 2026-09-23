//! Shared fold from recorded lifecycle facts into reporter/dashboard blocks.
//! Evidence adapters live in their owning crates; scope-path derivation lives
//! only here so consumers never need to know today's scope vocabulary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePhase {
    Setup,
    Teardown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatus {
    Passed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleScopeSegment {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleFlowOrigin {
    pub name: String,
    pub invocation: String,
    pub inner_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStepOutcome {
    Ok,
    Error,
    Timeout,
    Skipped { reason: String },
}

impl LifecycleStepOutcome {
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Error | Self::Timeout)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleStep {
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub uses: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<LifecycleFlowOrigin>,
    pub outcome: LifecycleStepOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleBlock {
    pub phase: LifecyclePhase,
    pub scope: Vec<LifecycleScopeSegment>,
    pub status: LifecycleStatus,
    pub started_at: String,
    pub duration_ms: u64,
    pub steps: Vec<LifecycleStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failing_step: Option<usize>,
}

impl LifecycleBlock {
    /// Human-facing path without knowledge of any particular kind/depth.
    pub fn scope_path(&self) -> String {
        if self.scope.is_empty() {
            return "leaf".to_string();
        }
        self.scope
            .iter()
            .map(|segment| format!("{}:{}", segment.kind, segment.id))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// Whether this block has the scope encoded by lifecycle evidence fields.
    ///
    /// Static hook-chain reporting uses this to pair declarations with the
    /// blocks that actually ran without duplicating the kind-to-segment map.
    pub fn matches_evidence_scope(
        &self,
        criterion_id: Option<&str>,
        check_id: Option<&str>,
        fixture_name: Option<&str>,
    ) -> bool {
        self.scope == scope_path(criterion_id, check_id, fixture_name)
    }
}

/// One normalized evidence fact consumed by [`LifecycleFold`].
pub enum LifecycleEvent<'a> {
    Started {
        phase: LifecyclePhase,
        criterion_id: Option<&'a str>,
        check_id: Option<&'a str>,
        fixture_name: Option<&'a str>,
        started_at: String,
        timestamp_ms: i64,
    },
    StepStarted {
        phase: LifecyclePhase,
        criterion_id: Option<&'a str>,
        check_id: Option<&'a str>,
        fixture_name: Option<&'a str>,
        index: u32,
        uses: &'a str,
        flow: Option<LifecycleFlowOrigin>,
        timestamp_ms: i64,
    },
    Observation {
        phase: LifecyclePhase,
        criterion_id: Option<&'a str>,
        check_id: Option<&'a str>,
        fixture_name: Option<&'a str>,
    },
    StepFinished {
        phase: LifecyclePhase,
        criterion_id: Option<&'a str>,
        check_id: Option<&'a str>,
        fixture_name: Option<&'a str>,
        index: u32,
        outcome: LifecycleStepOutcome,
        detail: Option<String>,
        timestamp_ms: i64,
    },
    Finished {
        phase: LifecyclePhase,
        criterion_id: Option<&'a str>,
        check_id: Option<&'a str>,
        fixture_name: Option<&'a str>,
        aborted: bool,
        timestamp_ms: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecycleLocation {
    pub block: usize,
    pub step: Option<usize>,
}

#[derive(Debug, Default)]
pub struct LifecycleFold {
    blocks: Vec<FoldBlock>,
}

#[derive(Debug)]
struct FoldBlock {
    block: LifecycleBlock,
    started_ms: i64,
    finished: bool,
    step_started_ms: Vec<i64>,
    step_finished: Vec<bool>,
}

impl LifecycleFold {
    pub fn push(&mut self, event: LifecycleEvent<'_>) -> Option<LifecycleLocation> {
        match event {
            LifecycleEvent::Started {
                phase,
                criterion_id,
                check_id,
                fixture_name,
                started_at,
                timestamp_ms,
            } => {
                let scope = scope_path(criterion_id, check_id, fixture_name);
                self.blocks.push(FoldBlock {
                    block: LifecycleBlock {
                        phase,
                        scope,
                        status: LifecycleStatus::Aborted,
                        started_at,
                        duration_ms: 0,
                        steps: Vec::new(),
                        failing_step: None,
                    },
                    started_ms: timestamp_ms,
                    finished: false,
                    step_started_ms: Vec::new(),
                    step_finished: Vec::new(),
                });
                Some(LifecycleLocation {
                    block: self.blocks.len() - 1,
                    step: None,
                })
            }
            LifecycleEvent::StepStarted {
                phase,
                criterion_id,
                check_id,
                fixture_name,
                index,
                uses,
                flow,
                timestamp_ms,
            } => {
                let block = self.active_block(phase, criterion_id, check_id, fixture_name)?;
                let folded = &mut self.blocks[block];
                folded.block.steps.push(LifecycleStep {
                    index,
                    id: None,
                    uses: uses.to_string(),
                    flow,
                    outcome: LifecycleStepOutcome::Ok,
                    detail: None,
                    duration_ms: 0,
                });
                folded.step_started_ms.push(timestamp_ms);
                folded.step_finished.push(false);
                Some(LifecycleLocation {
                    block,
                    step: Some(folded.block.steps.len() - 1),
                })
            }
            LifecycleEvent::Observation {
                phase,
                criterion_id,
                check_id,
                fixture_name,
            } => self
                .active_block(phase, criterion_id, check_id, fixture_name)
                .map(|block| LifecycleLocation { block, step: None }),
            LifecycleEvent::StepFinished {
                phase,
                criterion_id,
                check_id,
                fixture_name,
                index,
                outcome,
                detail,
                timestamp_ms,
            } => {
                let block = self.active_block(phase, criterion_id, check_id, fixture_name)?;
                let folded = &mut self.blocks[block];
                let step = folded
                    .block
                    .steps
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(position, candidate)| {
                        candidate.index == index && !folded.step_finished[*position]
                    })
                    .map(|(position, _)| position)?;
                folded.block.steps[step].outcome = outcome;
                folded.block.steps[step].detail = detail;
                folded.block.steps[step].duration_ms =
                    elapsed(folded.step_started_ms[step], timestamp_ms);
                folded.step_finished[step] = true;
                if folded.block.steps[step].outcome.is_failure()
                    && folded.block.failing_step.is_none()
                {
                    folded.block.failing_step = Some(step);
                }
                Some(LifecycleLocation {
                    block,
                    step: Some(step),
                })
            }
            LifecycleEvent::Finished {
                phase,
                criterion_id,
                check_id,
                fixture_name,
                aborted,
                timestamp_ms,
            } => {
                let block = self.active_block(phase, criterion_id, check_id, fixture_name)?;
                let folded = &mut self.blocks[block];
                folded.block.duration_ms = elapsed(folded.started_ms, timestamp_ms);
                folded.block.status = if aborted && phase == LifecyclePhase::Setup {
                    LifecycleStatus::Aborted
                } else if folded.block.failing_step.is_some() {
                    LifecycleStatus::Failed
                } else if aborted {
                    LifecycleStatus::Aborted
                } else {
                    LifecycleStatus::Passed
                };
                folded.finished = true;
                Some(LifecycleLocation { block, step: None })
            }
        }
    }

    pub fn into_blocks(self) -> Vec<LifecycleBlock> {
        self.blocks.into_iter().map(|folded| folded.block).collect()
    }

    fn active_block(
        &self,
        phase: LifecyclePhase,
        criterion_id: Option<&str>,
        check_id: Option<&str>,
        fixture_name: Option<&str>,
    ) -> Option<usize> {
        let scope = scope_path(criterion_id, check_id, fixture_name);
        self.blocks.iter().rposition(|candidate| {
            !candidate.finished && candidate.block.phase == phase && candidate.block.scope == scope
        })
    }
}

fn elapsed(started_ms: i64, finished_ms: i64) -> u64 {
    finished_ms
        .saturating_sub(started_ms)
        .try_into()
        .unwrap_or(0)
}

/// The sole kind→segment derivation. Renderers only walk the resulting path.
fn scope_path(
    criterion_id: Option<&str>,
    check_id: Option<&str>,
    fixture_name: Option<&str>,
) -> Vec<LifecycleScopeSegment> {
    [
        ("criterion", criterion_id),
        ("check", check_id),
        ("fixture", fixture_name),
    ]
    .into_iter()
    .filter_map(|(kind, id)| {
        id.map(|id| LifecycleScopeSegment {
            kind: kind.to_string(),
            id: id.to_string(),
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_path_is_generic_for_display() {
        let block = LifecycleBlock {
            phase: LifecyclePhase::Setup,
            scope: vec![
                LifecycleScopeSegment {
                    kind: "future-a".into(),
                    id: "1".into(),
                },
                LifecycleScopeSegment {
                    kind: "future-b".into(),
                    id: "2".into(),
                },
                LifecycleScopeSegment {
                    kind: "future-c".into(),
                    id: "3".into(),
                },
            ],
            status: LifecycleStatus::Passed,
            started_at: "t".into(),
            duration_ms: 0,
            steps: vec![],
            failing_step: None,
        };
        assert_eq!(block.scope_path(), "future-a:1 / future-b:2 / future-c:3");
    }

    #[test]
    fn fold_records_every_current_scope_and_an_aborted_failing_step() {
        let scopes = [
            (None, None, None),
            (Some("AC-1"), None, None),
            (Some("AC-1"), Some("AC-1.1"), None),
            (None, Some("AC-1.1"), Some("db")),
        ];
        let mut fold = LifecycleFold::default();
        for (ordinal, (criterion_id, check_id, fixture_name)) in scopes.into_iter().enumerate() {
            fold.push(LifecycleEvent::Started {
                phase: LifecyclePhase::Setup,
                criterion_id,
                check_id,
                fixture_name,
                started_at: format!("t{ordinal}"),
                timestamp_ms: ordinal as i64 * 10,
            });
            fold.push(LifecycleEvent::Finished {
                phase: LifecyclePhase::Setup,
                criterion_id,
                check_id,
                fixture_name,
                aborted: false,
                timestamp_ms: ordinal as i64 * 10 + 5,
            });
        }
        fold.push(LifecycleEvent::Started {
            phase: LifecyclePhase::Setup,
            criterion_id: None,
            check_id: None,
            fixture_name: None,
            started_at: "failed".into(),
            timestamp_ms: 100,
        });
        fold.push(LifecycleEvent::StepStarted {
            phase: LifecyclePhase::Setup,
            criterion_id: None,
            check_id: None,
            fixture_name: None,
            index: 7,
            uses: "cli/invoke",
            flow: Some(LifecycleFlowOrigin {
                name: "prepare".into(),
                invocation: "call-prepare".into(),
                inner_index: 2,
                iteration: Some(3),
            }),
            timestamp_ms: 101,
        });
        fold.push(LifecycleEvent::StepFinished {
            phase: LifecyclePhase::Setup,
            criterion_id: None,
            check_id: None,
            fixture_name: None,
            index: 7,
            outcome: LifecycleStepOutcome::Error,
            detail: Some("boom".into()),
            timestamp_ms: 104,
        });
        fold.push(LifecycleEvent::Finished {
            phase: LifecyclePhase::Setup,
            criterion_id: None,
            check_id: None,
            fixture_name: None,
            aborted: true,
            timestamp_ms: 105,
        });

        let blocks = fold.into_blocks();
        assert_eq!(blocks[0].scope, vec![]);
        assert_eq!(blocks[1].scope_path(), "criterion:AC-1");
        assert_eq!(blocks[2].scope_path(), "criterion:AC-1 / check:AC-1.1");
        assert_eq!(blocks[3].scope_path(), "check:AC-1.1 / fixture:db");
        assert_eq!(blocks[0].status, LifecycleStatus::Passed);
        assert_eq!(blocks[4].status, LifecycleStatus::Aborted);
        assert_eq!(blocks[4].failing_step, Some(0));
        assert_eq!(blocks[4].steps[0].detail.as_deref(), Some("boom"));
        assert_eq!(
            blocks[4].steps[0].flow,
            Some(LifecycleFlowOrigin {
                name: "prepare".into(),
                invocation: "call-prepare".into(),
                inner_index: 2,
                iteration: Some(3),
            })
        );
    }
}
