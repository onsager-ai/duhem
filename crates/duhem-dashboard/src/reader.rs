//! Read-only evidence reader: walks an evidence directory and builds
//! the API shapes in [`crate::model`] from `trace.jsonl`.
//!
//! Layout (set by `duhem-cli`): a single-leaf `duhem run` lands at
//! `<evidence-dir>/<run-id>/`; a manifest run (#49) lands each leaf at
//! `<evidence-dir>/<leaf-name>/<run-id>/`. A directory is a run dir
//! iff it contains `trace.jsonl`.
//!
//! Parsing is *lenient at the tail*: an in-progress run (#84) may have
//! a final line still being appended, so only complete
//! `\n`-terminated lines are parsed (the writer's line-atomicity
//! guarantee makes this safe). Everything else stays strict — bad
//! JSON or a non-monotonic `seq` on a complete line is an error, same
//! posture as `duhem_evidence::Trace`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use duhem_evidence::{Event, EventPayload, ObservationValue, VerdictState};
use duhem_judge::{RunVerdict, aggregate_run_set};
use thiserror::Error;

use crate::model::{
    ArtifactRef, CheckDetail, CheckRef, CriterionDetail, EntryKind, RunDetail, RunsListEntry,
};

#[derive(Debug, Error)]
pub enum ReaderError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("trace parse error in {run_id} line {line}: {source}")]
    Parse {
        run_id: String,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("trace seq not monotonic in {run_id} line {line}: expected {expected}, got {got}")]
    SeqNotMonotonic {
        run_id: String,
        line: usize,
        expected: u64,
        got: u64,
    },
    #[error("artifact id {0:?} is not a 64-char lowercase hex sha-256")]
    BadArtifactId(String),
}

/// All complete events of one run, plus what the tail told us.
#[derive(Debug, Clone)]
pub struct RunEvidence {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub events: Vec<Event>,
    /// `true` iff a `run_finished` event is present. The inverse is
    /// the #84 "in progress" predicate.
    pub finished: bool,
}

/// Parse the complete (`\n`-terminated) lines of `trace.jsonl` under
/// `run_dir`. A trailing partial line is ignored, not an error.
pub fn load_run(run_dir: &Path) -> Result<RunEvidence, ReaderError> {
    let run_id = dir_name(run_dir);
    let mut raw = Vec::new();
    fs::File::open(run_dir.join("trace.jsonl"))?.read_to_end(&mut raw)?;
    // Drop the partial tail: everything after the last '\n' is a line
    // still being appended by a live writer.
    let complete = match raw.iter().rposition(|&b| b == b'\n') {
        Some(pos) => &raw[..=pos],
        None => &[][..],
    };

    let mut events = Vec::new();
    let mut finished = false;
    for (idx, line) in complete.split(|&b| b == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        let evt: Event = serde_json::from_slice(line).map_err(|e| ReaderError::Parse {
            run_id: run_id.clone(),
            line: idx + 1,
            source: e,
        })?;
        let expected = events.len() as u64;
        if evt.seq != expected {
            return Err(ReaderError::SeqNotMonotonic {
                run_id: run_id.clone(),
                line: idx + 1,
                expected,
                got: evt.seq,
            });
        }
        if matches!(evt.payload, EventPayload::RunFinished { .. }) {
            finished = true;
        }
        events.push(evt);
    }

    Ok(RunEvidence {
        run_id,
        run_dir: run_dir.to_path_buf(),
        events,
        finished,
    })
}

impl RunEvidence {
    pub fn started_at(&self) -> Option<DateTime<Utc>> {
        self.events.first().map(|e| e.ts)
    }

    /// Wall-clock span of the trace. Only meaningful once finished.
    pub fn duration_ms(&self) -> Option<u64> {
        if !self.finished {
            return None;
        }
        let first = self.events.first()?.ts;
        let last = self.events.last()?.ts;
        u64::try_from((last - first).num_milliseconds()).ok()
    }

    /// The judge's recorded run verdict, if the run has finished.
    pub fn verdict(&self) -> Option<VerdictState> {
        self.events.iter().rev().find_map(|e| match &e.payload {
            EventPayload::RunFinished { verdict } => Some(*verdict),
            _ => None,
        })
    }

    /// Verification name: prefer `manifest.json`'s `definition_path`,
    /// fall back to the `run_started` event's `verification_path`,
    /// then to the run dir name. The path→name rule mirrors the CLI's
    /// `leaf_name`: a `duhem.yml` leaf is named by its parent dir,
    /// anything else by its file stem.
    pub fn verification(&self) -> String {
        if let Ok(bytes) = fs::read(self.run_dir.join("manifest.json"))
            && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(path) = v.get("definition_path").and_then(|p| p.as_str())
        {
            return verification_name(path);
        }
        for e in &self.events {
            if let EventPayload::RunStarted {
                verification_path, ..
            } = &e.payload
            {
                return verification_name(verification_path);
            }
        }
        self.run_id.clone()
    }
}

/// `leaf_name` twin (see `duhem-cli`): `duhem.yml` / `duhem.yaml` →
/// parent dir name; otherwise the file stem.
fn verification_name(definition_path: &str) -> String {
    let path = Path::new(definition_path);
    let file_name = path.file_name().and_then(|n| n.to_str());
    if matches!(file_name, Some("duhem.yml") | Some("duhem.yaml"))
        && let Some(parent) = path.parent()
        && let Some(name) = parent.file_name().and_then(|n| n.to_str())
        && !name.is_empty()
        && name != "."
        && name != ".."
    {
        return name.to_string();
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("verification")
        .to_string()
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn is_run_dir(dir: &Path) -> bool {
    dir.join("trace.jsonl").is_file()
}

/// Read-only view over one evidence directory. Stateless: every call
/// re-reads the filesystem (the MVP's hot-reload posture from #53 —
/// no cache, no invalidation bug).
#[derive(Debug, Clone)]
pub struct EvidenceReader {
    root: PathBuf,
}

impl EvidenceReader {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Find the run dir for `run_id`: directly under the root, or one
    /// verification-directory level down (#49 layout).
    pub fn locate_run(&self, run_id: &str) -> Option<PathBuf> {
        // Run ids are ULIDs minted by the engine; reject path-shaped
        // input before joining it to the root.
        if run_id.is_empty()
            || !run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return None;
        }
        let direct = self.root.join(run_id);
        if is_run_dir(&direct) {
            return Some(direct);
        }
        for group in read_subdirs(&self.root) {
            let nested = group.join(run_id);
            if is_run_dir(&nested) {
                return Some(nested);
            }
        }
        None
    }

    /// `GET /api/runs`: leaf rows for runs directly under the root,
    /// run-set rows (with nested leaf children) for verification
    /// directories. Unreadable run dirs are skipped — one corrupt
    /// trace must not take down the whole list.
    pub fn list(&self) -> Vec<RunsListEntry> {
        let mut entries = Vec::new();
        for dir in read_subdirs(&self.root) {
            if is_run_dir(&dir) {
                if let Ok(run) = load_run(&dir) {
                    entries.push(leaf_entry(&run, None));
                }
            } else {
                let group_name = dir_name(&dir);
                let mut children: Vec<RunsListEntry> = read_subdirs(&dir)
                    .into_iter()
                    .filter(|d| is_run_dir(d))
                    .filter_map(|d| load_run(&d).ok())
                    .map(|run| leaf_entry(&run, Some(&group_name)))
                    .collect();
                if children.is_empty() {
                    continue;
                }
                sort_newest_first(&mut children);
                entries.push(group_entry(group_name, children));
            }
        }
        sort_newest_first(&mut entries);
        entries
    }

    pub fn run_detail(&self, run_id: &str) -> Result<Option<RunDetail>, ReaderError> {
        let Some(dir) = self.locate_run(run_id) else {
            return Ok(None);
        };
        let run = load_run(&dir)?;
        Ok(Some(build_run_detail(&run, self.group_of(&dir))))
    }

    pub fn check_detail(
        &self,
        run_id: &str,
        criterion_id: &str,
        check_id: &str,
    ) -> Result<Option<CheckDetail>, ReaderError> {
        let Some(dir) = self.locate_run(run_id) else {
            return Ok(None);
        };
        let run = load_run(&dir)?;
        Ok(build_check_detail(&run, criterion_id, check_id))
    }

    /// Raw artifact bytes by content address, with a sniffed
    /// content-type. The hex check mirrors `Trace::read_blob`'s
    /// traversal guard.
    pub fn artifact(
        &self,
        run_id: &str,
        artifact_id: &str,
    ) -> Result<Option<(Vec<u8>, &'static str)>, ReaderError> {
        if !is_valid_sha256_hex(artifact_id) {
            return Err(ReaderError::BadArtifactId(artifact_id.to_string()));
        }
        let Some(dir) = self.locate_run(run_id) else {
            return Ok(None);
        };
        let path = dir.join("blobs").join(artifact_id);
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = fs::read(path)?;
        let mime = sniff_content_type(&bytes);
        Ok(Some((bytes, mime)))
    }

    /// Verification-group name when the run dir is nested (#49).
    fn group_of(&self, run_dir: &Path) -> Option<String> {
        let parent = run_dir.parent()?;
        (parent != self.root).then(|| dir_name(parent))
    }
}

fn read_subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn sort_newest_first(entries: &mut [RunsListEntry]) {
    // Newest first; runs with no parseable start (empty trace) sink
    // to the bottom — a bare `Reverse(Option)` would float them to
    // the top instead, since `None < Some` pre-reversal.
    entries.sort_by_key(|e| (e.started_at.is_none(), std::cmp::Reverse(e.started_at)));
}

fn leaf_entry(run: &RunEvidence, group: Option<&str>) -> RunsListEntry {
    RunsListEntry {
        run_id: run.run_id.clone(),
        verification: group
            .map(str::to_string)
            .unwrap_or_else(|| run.verification()),
        started_at: run.started_at(),
        duration_ms: run.duration_ms(),
        verdict: run.verdict(),
        kind: EntryKind::Leaf,
        live: !run.finished,
        children: None,
    }
}

/// Roll a verification directory up into a run-set row. The rollup
/// state is the judge's `aggregate_run_set` fold over the *recorded*
/// child verdicts — the dashboard never invents a verdict. While any
/// child is still live the rollup is withheld (`None`).
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

fn build_run_detail(run: &RunEvidence, group: Option<String>) -> RunDetail {
    let mut inputs = serde_json::Map::new();
    let mut setup_aborted = false;
    // First-seen orderings from the trace itself.
    let mut criterion_order: Vec<String> = Vec::new();
    let mut checks_by_criterion: Vec<(String, Vec<CheckRef>)> = Vec::new();
    // A check belongs to exactly one criterion (replay rejects
    // conflicting mappings outright); first `step_started` wins here
    // so a malformed trace can't smear one check's verdict across
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
                // a malformed trace and must not duplicate the row.
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
        run_id: run.run_id.clone(),
        verification: group.unwrap_or_else(|| run.verification()),
        started_at: run.started_at(),
        inputs,
        verdict: run_verdict,
        live: !run.finished,
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
                url: format!("/api/runs/{}/artifact/{}", run.run_id, blob_sha256),
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
/// Blobs carry no media type in the trace (the observation's
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
