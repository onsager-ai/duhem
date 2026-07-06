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
    ArtifactRef, CheckDetail, CheckRef, CriterionDetail, EntryKind, RunDetail, RunsListEntry,
};

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
        Ok(build_check_detail(&run, criterion_id, check_id))
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
            EventPayload::RunStarted { inputs: i, .. } => {
                inputs = i.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
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
        criteria,
    }
}

fn build_check_detail(
    run: &RunEvidence,
    criterion_id: &str,
    check_id: &str,
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
        timeline,
        artifacts,
    })
}

fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Cheap content sniff for artifact serving and export extensions.
/// Blobs carry no media type in the stream (the observation's
/// `output_name` is a label, not a MIME), so the bytes decide.
pub fn sniff_content_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if serde_json::from_slice::<serde_json::Value>(bytes).is_ok() {
        "application/json"
    } else if std::str::from_utf8(bytes).is_ok() {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

/// Export-side file extension for a sniffed content type.
pub fn extension_for(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "application/json" => "json",
        m if m.starts_with("text/plain") => "txt",
        _ => "bin",
    }
}
