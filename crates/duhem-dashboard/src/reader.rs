//! Read-only evidence reader: builds the API shapes in
//! [`crate::model`] from the evidence store (#189).
//!
//! The reader holds a **read-only** store handle — the read-only-lens
//! invariant is enforced by the connection (SQLite `mode=ro`), not by
//! discipline. Every call re-queries the store (the MVP's hot-reload
//! posture from #53 — no cache, no invalidation bug).
//!
//! Runs are listed flat from the store and grouped by verification
//! name: a verification with more than one recorded run renders as a
//! run-set row with its runs as children (the pre-#190 stand-in for
//! real scoping; it also seeds the ② VD-over-time altitude from
//! #188). The rollup verdict is the judge's `aggregate_run_set` fold
//! over recorded verdicts — the dashboard never invents a verdict.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use duhem_evidence::{Event, EventPayload, ObservationValue, RunRecord, Store, VerdictState};
use duhem_judge::{RunVerdict, aggregate_run_set};
use thiserror::Error;

use crate::model::{
    ArtifactRef, AssertionDiff, CheckDetail, CheckDiff, CheckRef, CriterionDetail, CriterionDiff,
    CriterionHistory, EntryKind, FailingAssertion, FailingCheck, FailingRequest, FailureEnvelope,
    HistoryRun, RunDetail, RunDiff, RunSide, RunsListEntry, SpanModel, VerificationHistory,
};

mod mime;
pub use mime::{extension_for, sniff_content_type};

#[derive(Debug, Error)]
pub enum ReaderError {
    #[error("store error: {0}")]
    Store(#[from] duhem_evidence::StoreError),
    #[error("artifact id {0:?} is not a 64-char lowercase hex sha-256")]
    BadArtifactId(String),
}

/// All events of one run plus its store header — the unit the API
/// models are folded from.
#[derive(Debug, Clone)]
pub struct RunEvidence {
    pub record: RunRecord,
    pub events: Vec<Event>,
}

impl RunEvidence {
    /// `true` iff the run's verdict has landed. The inverse is the
    /// #84 "in progress" predicate.
    pub fn finished(&self) -> bool {
        self.record.verdict.is_some()
    }

    pub fn started_at(&self) -> Option<DateTime<Utc>> {
        Some(self.record.started_at)
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.record.duration_ms
    }

    /// The judge's recorded run verdict, if the run has finished.
    pub fn verdict(&self) -> Option<VerdictState> {
        self.record.verdict
    }

    /// Verification name derived from the recorded definition path.
    /// The path→name rule mirrors the CLI's `leaf_name`: a `duhem.yml`
    /// leaf is named by its parent dir, anything else by its file
    /// stem.
    pub fn verification(&self) -> String {
        verification_name(&self.record.verification)
    }
}

pub use duhem_evidence::verification_name;

/// Read-only view over the evidence store. Clone-cheap (`Arc`).
#[derive(Clone)]
pub struct EvidenceReader {
    store: Arc<dyn Store>,
}

impl EvidenceReader {
    /// Wrap a store handle. Callers pass a read-only handle
    /// (`SqliteStore::open_read_only`) — the dashboard never needs a
    /// writable one.
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    async fn load(&self, run_id: &str) -> Result<Option<RunEvidence>, ReaderError> {
        let Some(record) = self.store.get_run(run_id).await? else {
            return Ok(None);
        };
        let events = self.store.run_events(run_id).await?;
        Ok(Some(RunEvidence { record, events }))
    }

    /// `GET /api/runs`: leaf rows for verifications with one recorded
    /// run, run-set rows (children newest-first) for verifications
    /// with several. Unreadable runs are skipped — one corrupt run
    /// must not take down the whole list.
    pub async fn list(&self) -> Result<Vec<RunsListEntry>, ReaderError> {
        let records = self.store.list_runs().await?;

        // Group by verification name, preserving newest-first order.
        let mut groups: Vec<(String, Vec<RunsListEntry>)> = Vec::new();
        for record in records {
            let name = verification_name(&record.verification);
            let leaf = leaf_entry_from_record(&record);
            match groups.iter_mut().find(|(n, _)| *n == name) {
                Some((_, rows)) => rows.push(leaf),
                None => groups.push((name, vec![leaf])),
            }
        }

        let mut entries: Vec<RunsListEntry> = groups
            .into_iter()
            .map(|(name, mut rows)| {
                if rows.len() == 1 {
                    rows.pop().expect("one row")
                } else {
                    sort_newest_first(&mut rows);
                    group_entry(name, rows)
                }
            })
            .collect();
        sort_newest_first(&mut entries);
        Ok(entries)
    }

    pub async fn run_detail(&self, run_id: &str) -> Result<Option<RunDetail>, ReaderError> {
        let Some(run) = self.load(run_id).await? else {
            return Ok(None);
        };
        Ok(Some(build_run_detail(&run)))
    }

    pub async fn check_detail(
        &self,
        run_id: &str,
        criterion_id: &str,
        check_id: &str,
    ) -> Result<Option<CheckDetail>, ReaderError> {
        let Some(run) = self.load(run_id).await? else {
            return Ok(None);
        };
        let spans = self
            .store
            .check_spans(run_id, check_id)
            .await?
            .into_iter()
            .map(|s| SpanModel {
                seq: s.seq,
                layer: s.layer,
                ok: s.ok,
                detail: s.detail,
            })
            .collect();
        Ok(build_check_detail(&run, criterion_id, check_id, spans))
    }

    /// `GET /api/verifications/:name/history` (② VD-over-time, #193):
    /// the verification's runs newest-first plus each criterion's
    /// verdict across them, straight from the #190 history queries.
    pub async fn verification_history(
        &self,
        name: &str,
    ) -> Result<Option<VerificationHistory>, ReaderError> {
        let records = self.store.verification_history(name).await?;
        if records.is_empty() {
            return Ok(None);
        }
        let runs: Vec<HistoryRun> = records
            .iter()
            .map(|r| HistoryRun {
                run_id: r.run_id.clone(),
                started_at: Some(r.started_at),
                verdict: r.verdict,
                duration_ms: r.duration_ms,
            })
            .collect();
        let entries = self.store.criterion_history(name).await?;
        // Criterion spine in first-seen order (stable across runs);
        // one verdict slot per run column, `None` where the criterion
        // did not appear on that run.
        let mut criteria: Vec<CriterionHistory> = Vec::new();
        for e in &entries {
            if !criteria.iter().any(|c| c.criterion_id == e.criterion_id) {
                criteria.push(CriterionHistory {
                    criterion_id: e.criterion_id.clone(),
                    verdicts: vec![None; runs.len()],
                });
            }
        }
        for e in &entries {
            let col = runs.iter().position(|r| r.run_id == e.run_id);
            let row = criteria
                .iter_mut()
                .find(|c| c.criterion_id == e.criterion_id);
            if let (Some(col), Some(row)) = (col, row) {
                row.verdicts[col] = Some(e.verdict);
            }
        }
        Ok(Some(VerificationHistory {
            name: name.to_string(),
            runs,
            criteria,
        }))
    }

    /// `GET /api/runs/:run_id/diff` (#211): compare a run against its
    /// baseline — the most recent prior *passing* run of the same
    /// verification + target (last-pass), or the explicit
    /// `baseline_override` run. Surfaces recorded verdict/assertion
    /// transitions per criterion/check plus each check's artifacts on
    /// both sides. `None` (the outer option) only when the *current*
    /// run doesn't exist; a missing baseline is `RunDiff.baseline =
    /// None` — the view says "no passing baseline", it never diffs two
    /// failures by default.
    pub async fn run_diff(
        &self,
        run_id: &str,
        baseline_override: Option<&str>,
    ) -> Result<Option<RunDiff>, ReaderError> {
        let Some(current) = self.load(run_id).await? else {
            return Ok(None);
        };
        let baseline = match baseline_override {
            Some(bid) => self.load(bid).await?,
            None => self.resolve_baseline(&current.record).await?,
        };

        let cur_proj = project_run(&current);
        let base_proj = baseline.as_ref().map(project_run);
        let criteria = diff_criteria(&cur_proj, base_proj.as_ref());

        Ok(Some(RunDiff {
            current: run_side(&current.record),
            baseline: baseline.as_ref().map(|b| run_side(&b.record)),
            criteria,
        }))
    }

    /// Last-pass baseline: the most recent prior run of the same
    /// verification whose recorded verdict is `pass` and whose target
    /// repo matches. Reaches back over a whole failing streak to the
    /// last-known-good run — the regression question ("what changed
    /// since it last worked"). `None` when the verification has never
    /// passed against this target.
    async fn resolve_baseline(
        &self,
        current: &RunRecord,
    ) -> Result<Option<RunEvidence>, ReaderError> {
        let name = verification_name(&current.verification);
        let history = self.store.verification_history(&name).await?;
        // `verification_history` is newest-first, so the first prior
        // passing run with a matching target is the most recent one.
        let picked = history.into_iter().find(|r| {
            r.run_id != current.run_id
                && r.verdict == Some(VerdictState::Pass)
                && r.scope.target_repo == current.scope.target_repo
                && (r.started_at, &r.run_id) < (current.started_at, &current.run_id)
        });
        match picked {
            Some(rec) => self.load(&rec.run_id).await,
            None => Ok(None),
        }
    }

    /// `GET /api/runs/:run_id/failure` (#216): the machine-readable
    /// failure envelope — every non-passing check with its failing
    /// assertions, delivery-web layer chain, artifact URLs, and first
    /// failing request. Everything an agent needs to react to a `fail`
    /// without scraping the SPA. `None` only when the run doesn't
    /// exist; a passing run yields `failing: []`.
    pub async fn failure_envelope(
        &self,
        run_id: &str,
    ) -> Result<Option<FailureEnvelope>, ReaderError> {
        let Some(run) = self.load(run_id).await? else {
            return Ok(None);
        };
        let proj = project_run(&run);
        let mut failing = Vec::new();
        for c in &proj.checks {
            if c.verdict == Some(VerdictState::Pass) {
                continue;
            }
            failing.push(self.build_failing_check(run_id, c).await?);
        }
        Ok(Some(FailureEnvelope {
            run_id: run.record.run_id.clone(),
            verification: verification_name(&run.record.verification),
            verdict: run.record.verdict,
            failing,
        }))
    }

    /// `GET /api/runs/:run_id/failure/:crit::check` — the same envelope
    /// scoped to one check (an agent handling a specific failure).
    /// `None` when the run or that check doesn't exist.
    pub async fn failing_check(
        &self,
        run_id: &str,
        criterion_id: &str,
        check_id: &str,
    ) -> Result<Option<FailingCheck>, ReaderError> {
        let Some(run) = self.load(run_id).await? else {
            return Ok(None);
        };
        let proj = project_run(&run);
        let Some(c) = proj
            .checks
            .iter()
            .find(|c| c.check_id == check_id && c.criterion_id == criterion_id)
        else {
            return Ok(None);
        };
        Ok(Some(self.build_failing_check(run_id, c).await?))
    }

    async fn build_failing_check(
        &self,
        run_id: &str,
        c: &CheckProjection,
    ) -> Result<FailingCheck, ReaderError> {
        let layers = self
            .store
            .check_spans(run_id, &c.check_id)
            .await?
            .into_iter()
            .map(|s| s.layer)
            .collect();
        let assertions = c
            .assertions
            .iter()
            .filter(|(_, s, _)| *s != VerdictState::Pass)
            .map(|(i, s, d)| FailingAssertion {
                assertion_index: *i,
                state: *s,
                detail: d.clone(),
            })
            .collect();
        let first_failing_request = self.first_failing_request(&c.artifacts).await?;
        Ok(FailingCheck {
            criterion_id: c.criterion_id.clone(),
            check_id: c.check_id.clone(),
            verdict: c.verdict,
            layers,
            assertions,
            artifacts: c.artifacts.clone(),
            first_failing_request,
        })
    }

    /// The first request in the check's `capture/network` HAR whose
    /// response status is an error (≥ 400) — usually the request that
    /// broke. `None` when there's no network capture or no error.
    async fn first_failing_request(
        &self,
        artifacts: &[ArtifactRef],
    ) -> Result<Option<FailingRequest>, ReaderError> {
        let Some(net) = artifacts.iter().find(|a| a.kind == "capture/network") else {
            return Ok(None);
        };
        let Some(bytes) = self.store.get_blob(&net.id).await? else {
            return Ok(None);
        };
        let Ok(har) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return Ok(None);
        };
        let Some(entries) = har
            .get("log")
            .and_then(|l| l.get("entries"))
            .and_then(|e| e.as_array())
        else {
            return Ok(None);
        };
        for e in entries {
            let status = e
                .get("response")
                .and_then(|r| r.get("status"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if status < 400 {
                continue;
            }
            let req = |k: &str| {
                e.get("request")
                    .and_then(|r| r.get(k))
                    .and_then(serde_json::Value::as_str)
            };
            // Skip a malformed entry (missing method/url) rather than
            // emit an empty request that would mislead the agent.
            if let (Some(method), Some(url)) = (req("method"), req("url")) {
                return Ok(Some(FailingRequest {
                    method: method.to_string(),
                    url: url.to_string(),
                    status: status as u16,
                }));
            }
        }
        Ok(None)
    }

    /// The raw wire-format event stream for a run (NDJSON), if the
    /// run exists.
    pub async fn raw_events_jsonl(&self, run_id: &str) -> Result<Option<String>, ReaderError> {
        if self.store.get_run(run_id).await?.is_none() {
            return Ok(None);
        }
        let events = self.store.run_events(run_id).await?;
        Ok(Some(events_to_jsonl(&events)))
    }

    /// The recorded Verification Definition source snapshot for a run
    /// (raw YAML, spec #302), read from its `run_started` event.
    /// `None` when the run doesn't exist or predates the snapshot field
    /// (both surface as `404` on the API — the client only requests it
    /// when run detail reports `has_definition`).
    pub async fn run_definition(&self, run_id: &str) -> Result<Option<String>, ReaderError> {
        if self.store.get_run(run_id).await?.is_none() {
            return Ok(None);
        }
        let events = self.store.run_events(run_id).await?;
        Ok(events.iter().find_map(|e| match &e.payload {
            EventPayload::RunStarted { definition, .. } => definition.clone(),
            _ => None,
        }))
    }

    /// Raw artifact bytes by content address, with a sniffed
    /// content-type. The hex guard mirrors the store's own.
    pub async fn artifact(
        &self,
        _run_id: &str,
        artifact_id: &str,
    ) -> Result<Option<(Vec<u8>, &'static str)>, ReaderError> {
        if !is_valid_sha256_hex(artifact_id) {
            return Err(ReaderError::BadArtifactId(artifact_id.to_string()));
        }
        let Some(bytes) = self.store.get_blob(artifact_id).await? else {
            return Ok(None);
        };
        let mime = sniff_content_type(&bytes);
        Ok(Some((bytes, mime)))
    }
}

/// Serialize events back to the wire-format NDJSON (one event JSON
/// object per line) — the export/raw-trace shape, byte-compatible
/// with the retired `trace.jsonl` files.
pub fn events_to_jsonl(events: &[Event]) -> String {
    let mut out = String::new();
    for e in events {
        if let Ok(line) = serde_json::to_string(e) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

fn sort_newest_first(entries: &mut [RunsListEntry]) {
    // Newest first; runs with no parseable start sink to the bottom —
    // a bare `Reverse(Option)` would float them to the top instead,
    // since `None < Some` pre-reversal.
    entries.sort_by_key(|e| (e.started_at.is_none(), std::cmp::Reverse(e.started_at)));
}

fn leaf_entry_from_record(record: &RunRecord) -> RunsListEntry {
    RunsListEntry {
        run_id: record.run_id.clone(),
        verification: verification_name(&record.verification),
        started_at: Some(record.started_at),
        duration_ms: record.duration_ms,
        verdict: record.verdict,
        kind: EntryKind::Leaf,
        live: record.verdict.is_none(),
        children: None,
    }
}

/// Roll a verification's runs up into a run-set row. The rollup state
/// is the judge's `aggregate_run_set` fold over the *recorded* child
/// verdicts — the dashboard never invents a verdict. While any child
/// is still live the rollup is withheld (`None`).
fn group_entry(name: String, children: Vec<RunsListEntry>) -> RunsListEntry {
    let verdict = if children.iter().all(|c| c.verdict.is_some()) {
        let runs: Vec<RunVerdict> = children
            .iter()
            .map(|c| RunVerdict {
                state: c.verdict.expect("checked above"),
                // The fold reads only `state`; the children's criteria
                // are not re-materialized for a list row.
                criteria: Vec::new(),
            })
            .collect();
        Some(aggregate_run_set(runs).state)
    } else {
        None
    };
    let live = children.iter().any(|c| c.live);
    let started_at = children.iter().filter_map(|c| c.started_at).min();
    let duration_ms = children.iter().map(|c| c.duration_ms).sum::<Option<u64>>();
    RunsListEntry {
        run_id: name.clone(),
        verification: name,
        started_at,
        duration_ms,
        verdict,
        kind: EntryKind::RunSet,
        live,
        children: Some(children),
    }
}

fn build_run_detail(run: &RunEvidence) -> RunDetail {
    let mut inputs = serde_json::Map::new();
    let mut setup_aborted = false;
    let mut has_definition = false;
    // First-seen orderings from the event stream itself.
    let mut criterion_order: Vec<String> = Vec::new();
    let mut checks_by_criterion: Vec<(String, Vec<CheckRef>)> = Vec::new();
    // A check belongs to exactly one criterion (replay rejects
    // conflicting mappings outright); first `step_started` wins here
    // so a malformed stream can't smear one check's verdict across
    // criteria.
    let mut criterion_of_check: Vec<(String, String)> = Vec::new();
    let mut run_verdict = None;

    fn note_check(
        criterion_order: &mut Vec<String>,
        checks_by_criterion: &mut Vec<(String, Vec<CheckRef>)>,
        criterion_id: &str,
        check_id: &str,
    ) {
        let idx = match checks_by_criterion
            .iter()
            .position(|(c, _)| c == criterion_id)
        {
            Some(i) => i,
            None => {
                criterion_order.push(criterion_id.to_string());
                checks_by_criterion.push((criterion_id.to_string(), Vec::new()));
                checks_by_criterion.len() - 1
            }
        };
        let slot = &mut checks_by_criterion[idx].1;
        if !slot.iter().any(|c| c.id == check_id) {
            slot.push(CheckRef {
                id: check_id.to_string(),
                verdict: None,
            });
        }
    }

    for evt in &run.events {
        match &evt.payload {
            EventPayload::RunStarted {
                inputs: i,
                definition,
                ..
            } => {
                inputs = i.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                has_definition = definition.is_some();
            }
            EventPayload::SetupFinished { aborted } => setup_aborted = *aborted,
            EventPayload::StepStarted {
                criterion_id,
                check_id,
                ..
            } => {
                let owner = criterion_of_check
                    .iter()
                    .find(|(k, _)| k == check_id)
                    .map(|(_, c)| c.clone())
                    .unwrap_or_else(|| {
                        criterion_of_check.push((check_id.clone(), criterion_id.clone()));
                        criterion_id.clone()
                    });
                // Only the owning criterion lists the check; a
                // colliding `step_started` under another criterion is
                // a malformed stream and must not duplicate the row.
                if owner == *criterion_id {
                    note_check(
                        &mut criterion_order,
                        &mut checks_by_criterion,
                        criterion_id,
                        check_id,
                    );
                }
            }
            EventPayload::CheckFinished { check_id, verdict } => {
                let owner = criterion_of_check
                    .iter()
                    .find(|(k, _)| k == check_id)
                    .map(|(_, c)| c.clone());
                if let Some(owner) = owner
                    && let Some((_, checks)) =
                        checks_by_criterion.iter_mut().find(|(c, _)| *c == owner)
                    && let Some(check) = checks.iter_mut().find(|c| c.id == *check_id)
                {
                    check.verdict = Some(*verdict);
                }
            }
            EventPayload::CriterionFinished { criterion_id, .. }
                if !criterion_order.contains(criterion_id) =>
            {
                criterion_order.push(criterion_id.clone());
                checks_by_criterion.push((criterion_id.clone(), Vec::new()));
            }
            EventPayload::RunFinished { verdict } => run_verdict = Some(*verdict),
            _ => {}
        }
    }

    let criteria = criterion_order
        .iter()
        .map(|id| {
            let checks = checks_by_criterion
                .iter()
                .find(|(c, _)| c == id)
                .map(|(_, checks)| checks.clone())
                .unwrap_or_default();
            let verdict = run.events.iter().find_map(|e| match &e.payload {
                EventPayload::CriterionFinished {
                    criterion_id,
                    verdict,
                } if criterion_id == id => Some(*verdict),
                _ => None,
            });
            CriterionDetail {
                id: id.clone(),
                verdict,
                checks,
            }
        })
        .collect();

    RunDetail {
        run_id: run.record.run_id.clone(),
        verification: run.verification(),
        started_at: run.started_at(),
        inputs,
        verdict: run_verdict,
        live: !run.finished(),
        setup_aborted,
        has_definition,
        criteria,
    }
}

// ---- #211: run-to-run diff -----------------------------------------

fn run_side(r: &RunRecord) -> RunSide {
    RunSide {
        run_id: r.run_id.clone(),
        started_at: Some(r.started_at),
        verdict: r.verdict,
    }
}

/// A run folded into the projection the diff compares over: ordered
/// criteria + checks with verdicts, each check's assertions and blob
/// artifacts. One pass over the event stream (same folds as
/// `build_run_detail` / `build_check_detail`, collected together).
struct RunProjection {
    criteria: Vec<(String, Option<VerdictState>)>,
    checks: Vec<CheckProjection>,
}

struct CheckProjection {
    criterion_id: String,
    check_id: String,
    verdict: Option<VerdictState>,
    /// `(assertion_index, state, detail)` in recorded order.
    assertions: Vec<(u32, VerdictState, Option<String>)>,
    artifacts: Vec<ArtifactRef>,
}

fn project_run(run: &RunEvidence) -> RunProjection {
    let mut criteria: Vec<(String, Option<VerdictState>)> = Vec::new();
    let mut checks: Vec<CheckProjection> = Vec::new();
    // Positional attribution for blob observations: they belong to the
    // check whose `step_started` most recently opened (same rule as
    // `build_check_detail`), so trailing `capture/*` observations land
    // on their check.
    let mut current: Option<usize> = None;

    for evt in &run.events {
        match &evt.payload {
            EventPayload::StepStarted {
                criterion_id,
                check_id,
                ..
            } => {
                if !criteria.iter().any(|(c, _)| c == criterion_id) {
                    criteria.push((criterion_id.clone(), None));
                }
                let pos = match checks.iter().position(|c| &c.check_id == check_id) {
                    Some(p) => p,
                    None => {
                        checks.push(CheckProjection {
                            criterion_id: criterion_id.clone(),
                            check_id: check_id.clone(),
                            verdict: None,
                            assertions: Vec::new(),
                            artifacts: Vec::new(),
                        });
                        checks.len() - 1
                    }
                };
                current = Some(pos);
            }
            EventPayload::StepObservation {
                output_name,
                value: ObservationValue::Blob { blob_sha256 },
                ..
            } => {
                if let Some(pos) = current {
                    checks[pos].artifacts.push(ArtifactRef {
                        id: blob_sha256.clone(),
                        kind: output_name.clone(),
                        url: format!("/api/runs/{}/artifact/{}", run.record.run_id, blob_sha256),
                    });
                }
            }
            EventPayload::AssertionEvaluated {
                check_id,
                assertion_index,
                state,
                detail,
                ..
            } => {
                if let Some(pos) = checks.iter().position(|c| &c.check_id == check_id) {
                    checks[pos]
                        .assertions
                        .push((*assertion_index, *state, detail.clone()));
                }
            }
            EventPayload::CheckFinished { check_id, verdict } => {
                if let Some(pos) = checks.iter().position(|c| &c.check_id == check_id) {
                    checks[pos].verdict = Some(*verdict);
                }
            }
            EventPayload::CriterionFinished {
                criterion_id,
                verdict,
            } => {
                if let Some(entry) = criteria.iter_mut().find(|(c, _)| c == criterion_id) {
                    entry.1 = Some(*verdict);
                }
            }
            _ => {}
        }
    }
    RunProjection { criteria, checks }
}

/// Ordered union of criterion ids (current first, then baseline-only)
/// with per-criterion / per-check / per-assertion transitions. When
/// there is no baseline, `changed` is always `false` — the view shows
/// the "no passing baseline" state rather than painting everything as
/// changed.
fn diff_criteria(cur: &RunProjection, base: Option<&RunProjection>) -> Vec<CriterionDiff> {
    let has_base = base.is_some();
    let mut ids: Vec<String> = cur.criteria.iter().map(|(c, _)| c.clone()).collect();
    if let Some(b) = base {
        for (c, _) in &b.criteria {
            if !ids.contains(c) {
                ids.push(c.clone());
            }
        }
    }
    ids.into_iter()
        .map(|cid| {
            let cur_v = cur
                .criteria
                .iter()
                .find(|(c, _)| *c == cid)
                .and_then(|(_, v)| *v);
            let base_v = base
                .and_then(|b| b.criteria.iter().find(|(c, _)| *c == cid))
                .and_then(|(_, v)| *v);
            CriterionDiff {
                id: cid.clone(),
                baseline_verdict: base_v,
                current_verdict: cur_v,
                changed: has_base && base_v != cur_v,
                checks: diff_checks(&cid, cur, base),
            }
        })
        .collect()
}

fn diff_checks(
    criterion_id: &str,
    cur: &RunProjection,
    base: Option<&RunProjection>,
) -> Vec<CheckDiff> {
    let has_base = base.is_some();
    let mut ids: Vec<String> = cur
        .checks
        .iter()
        .filter(|c| c.criterion_id == criterion_id)
        .map(|c| c.check_id.clone())
        .collect();
    if let Some(b) = base {
        for c in b.checks.iter().filter(|c| c.criterion_id == criterion_id) {
            if !ids.contains(&c.check_id) {
                ids.push(c.check_id.clone());
            }
        }
    }
    ids.into_iter()
        .map(|cid| {
            let cur_c = cur.checks.iter().find(|c| c.check_id == cid);
            let base_c = base.and_then(|b| b.checks.iter().find(|c| c.check_id == cid));
            let cur_v = cur_c.and_then(|c| c.verdict);
            let base_v = base_c.and_then(|c| c.verdict);
            CheckDiff {
                id: cid,
                baseline_verdict: base_v,
                current_verdict: cur_v,
                changed: has_base && base_v != cur_v,
                assertions: diff_assertions(cur_c, base_c, has_base),
                baseline_artifacts: base_c.map(|c| c.artifacts.clone()).unwrap_or_default(),
                current_artifacts: cur_c.map(|c| c.artifacts.clone()).unwrap_or_default(),
            }
        })
        .collect()
}

fn diff_assertions(
    cur: Option<&CheckProjection>,
    base: Option<&CheckProjection>,
    has_base: bool,
) -> Vec<AssertionDiff> {
    let mut idxs: Vec<u32> = cur
        .map(|c| c.assertions.iter().map(|(i, _, _)| *i).collect())
        .unwrap_or_default();
    if let Some(b) = base {
        for (i, _, _) in &b.assertions {
            if !idxs.contains(i) {
                idxs.push(*i);
            }
        }
    }
    idxs.sort_unstable();
    idxs.into_iter()
        .map(|i| {
            let c = cur.and_then(|c| c.assertions.iter().find(|(j, _, _)| *j == i));
            let b = base.and_then(|b| b.assertions.iter().find(|(j, _, _)| *j == i));
            let cur_state = c.map(|(_, s, _)| *s);
            let base_state = b.map(|(_, s, _)| *s);
            let cur_detail = c.and_then(|(_, _, d)| d.clone());
            let base_detail = b.and_then(|(_, _, d)| d.clone());
            let changed = has_base && (base_state != cur_state || base_detail != cur_detail);
            AssertionDiff {
                assertion_index: i,
                baseline_state: base_state,
                current_state: cur_state,
                baseline_detail: base_detail,
                current_detail: cur_detail,
                changed,
            }
        })
        .collect()
}

fn build_check_detail(
    run: &RunEvidence,
    criterion_id: &str,
    check_id: &str,
    spans: Vec<SpanModel>,
) -> Option<CheckDetail> {
    // A check belongs to exactly one criterion — replay hard-errors
    // on conflicting mappings; the view applies the same first-wins
    // ownership so `assertion_evaluated` / `check_finished` (which
    // carry only `check_id`) can't be attributed to a colliding
    // criterion. No ownership record → no such pair.
    let owner = run.events.iter().find_map(|e| match &e.payload {
        EventPayload::StepStarted {
            criterion_id: c,
            check_id: k,
            ..
        } if k == check_id => Some(c.clone()),
        _ => None,
    });
    if owner.as_deref() != Some(criterion_id) {
        return None;
    }

    let mut timeline: Vec<Event> = Vec::new();
    let mut verdict = None;
    // `step_observation` / `step_finished` carry only `step_index`;
    // attribution is positional — they belong to the pair iff the most
    // recent `step_started` opened it.
    let mut in_pair = false;

    for evt in &run.events {
        match &evt.payload {
            EventPayload::StepStarted {
                criterion_id: c,
                check_id: k,
                ..
            } => {
                in_pair = c == criterion_id && k == check_id;
                if in_pair {
                    timeline.push(evt.clone());
                }
            }
            EventPayload::StepObservation { .. } | EventPayload::StepFinished { .. } if in_pair => {
                timeline.push(evt.clone());
            }
            EventPayload::AssertionEvaluated { check_id: k, .. } if k == check_id => {
                timeline.push(evt.clone());
            }
            EventPayload::CheckFinished {
                check_id: k,
                verdict: v,
            } if k == check_id => {
                verdict = Some(*v);
                timeline.push(evt.clone());
            }
            _ => {}
        }
    }

    let artifacts = timeline
        .iter()
        .filter_map(|evt| match &evt.payload {
            EventPayload::StepObservation {
                output_name,
                value: ObservationValue::Blob { blob_sha256 },
                ..
            } => Some(ArtifactRef {
                id: blob_sha256.clone(),
                kind: output_name.clone(),
                url: format!("/api/runs/{}/artifact/{}", run.record.run_id, blob_sha256),
            }),
            _ => None,
        })
        .collect();

    Some(CheckDetail {
        criterion_id: criterion_id.to_string(),
        check_id: check_id.to_string(),
        verdict,
        spans,
        timeline,
        artifacts,
    })
}

fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// `sniff_content_type` / `extension_for` moved to the `mime` submodule
// (re-exported above) to keep this file within the prod-token budget.
