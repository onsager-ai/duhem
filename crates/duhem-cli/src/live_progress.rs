//! Live terminal progress for `duhem run` (#299).
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
//! The renderer is a fold over the same evidence events the store
//! persists — no second progress model. A criterion "starts" at its
//! first event (there is no `criterion_started` on the wire; the
//! first `step_started` / assertion serves) and closes at
//! `criterion_finished` with the judge's verdict.

use std::collections::HashMap;
use std::io::Write;

use duhem_evidence::{Event, EventPayload};
use tokio::sync::mpsc::UnboundedReceiver;

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

/// Drain the progress channel to stderr until `run_finished` (or the
/// channel closes). Spawned alongside the engine's run future.
pub async fn render_to_stderr(rx: UnboundedReceiver<Event>, plan: Plan) {
    let mut err = std::io::stderr();
    render(rx, plan, &mut err).await;
}

/// The fold itself, writer-generic for tests. Write errors are
/// ignored — progress is advisory and must never disturb the run.
pub async fn render<W: Write>(mut rx: UnboundedReceiver<Event>, plan: Plan, out: &mut W) {
    let n = plan.criterion_ids.len();
    // criterion id → (ordinal shown at start, first-event timestamp)
    let mut started: HashMap<String, (usize, chrono::DateTime<chrono::Utc>)> = HashMap::new();

    while let Some(evt) = rx.recv().await {
        let ts = evt.ts;
        match &evt.payload {
            EventPayload::EnvUpStarted { .. } => {
                let _ = writeln!(out, "  env: up…");
            }
            EventPayload::EnvReady { ok, elapsed_ms, .. } => {
                let _ = if *ok {
                    writeln!(out, "  env: ready ({:.1}s)", *elapsed_ms as f64 / 1000.0)
                } else {
                    writeln!(
                        out,
                        "  env: NOT ready ({:.1}s)",
                        *elapsed_ms as f64 / 1000.0
                    )
                };
            }
            EventPayload::StepStarted { criterion_id, .. } => {
                begin(out, &mut started, criterion_id, n, ts);
            }
            EventPayload::AssertionEvaluated { check_id, .. }
            | EventPayload::CheckFinished { check_id, .. } => {
                if let Some(cid) = plan.check_owner.get(check_id) {
                    let owner = cid.clone();
                    begin(out, &mut started, &owner, n, ts);
                }
            }
            EventPayload::CriterionFinished {
                criterion_id,
                verdict,
            } => {
                // A criterion whose checks were all filtered out (or
                // stepless-and-instant) may finish without a start
                // line; give it one so every verdict line has context.
                begin(out, &mut started, criterion_id, n, ts);
                let mark = match verdict {
                    duhem_judge::VerdictState::Pass => "✔",
                    duhem_judge::VerdictState::Fail => "✘",
                    duhem_judge::VerdictState::Inconclusive(_) => "◐",
                };
                let secs = started
                    .get(criterion_id)
                    .map(|(_, t0)| (ts - *t0).num_milliseconds() as f64 / 1000.0)
                    .unwrap_or(0.0);
                let _ = writeln!(out, "{mark} {criterion_id} {verdict} ({secs:.1}s)");
            }
            EventPayload::RunFinished { .. } => break,
            _ => {}
        }
    }
}

/// Print the "criterion running" line the first time a criterion is
/// seen; idempotent afterwards.
fn begin<W: Write>(
    out: &mut W,
    started: &mut HashMap<String, (usize, chrono::DateTime<chrono::Utc>)>,
    criterion_id: &str,
    n: usize,
    ts: chrono::DateTime<chrono::Utc>,
) {
    if started.contains_key(criterion_id) {
        return;
    }
    let k = started.len() + 1;
    started.insert(criterion_id.to_string(), (k, ts));
    let _ = writeln!(out, "▶ {criterion_id} ({k}/{n})…");
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

    async fn rendered(events: Vec<Event>) -> String {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        for e in events {
            tx.send(e).unwrap();
        }
        drop(tx);
        let mut out = Vec::new();
        render(rx, plan(), &mut out).await;
        String::from_utf8(out).unwrap()
    }

    #[tokio::test]
    async fn per_criterion_lines_with_ordinals_verdicts_and_durations() {
        let out = rendered(vec![
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
        ])
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
        let out = rendered(vec![
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
                2510,
                EventPayload::EnvReady {
                    probe_kind: "http".into(),
                    ok: true,
                    elapsed_ms: 2500,
                },
            ),
            evt(
                3,
                2600,
                EventPayload::RunFinished {
                    verdict: VerdictState::Pass,
                },
            ),
        ])
        .await;
        assert!(out.contains("env: up…"), "{out}");
        assert!(out.contains("env: ready (2.5s)"), "{out}");
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
        render(rx, plan(), &mut out).await;
        drop(tx);
    }
}
