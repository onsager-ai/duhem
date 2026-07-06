//! Validated view over a run's event stream.
//!
//! A [`Trace`] is the replay input: the full, seq-validated event
//! sequence for one run. Since #189 the stream lives in the store —
//! `Trace::from_store` pulls it back out; `Trace::from_events`
//! validates an in-memory stream (exports, tests).
//!
//! `seq` monotonicity is enforced on construction (gap or backwards =
//! hard error). The writer guarantees this on write; the reader is
//! the second line of defense against out-of-tree-produced streams.

use thiserror::Error;

use crate::event::Event;
use crate::store::{Store, StoreError};

#[derive(Debug, Error)]
pub enum ReadError {
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error("parse error on line {line}: {source}")]
    Parse {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "seq {got} at position {line} is not monotonic (expected {expected}{})",
        match prev {
            Some(p) => format!(", after seq {p}"),
            None => String::new(),
        }
    )]
    SeqNotMonotonic {
        line: usize,
        prev: Option<u64>,
        expected: u64,
        got: u64,
    },
}

/// A validated event stream for one run, fully materialized. Run
/// traces are bounded by the number of steps in a Verification
/// Definition — small enough that an in-memory `Vec` beats streaming.
#[derive(Debug, Clone)]
pub struct Trace {
    events: Vec<Event>,
}

impl Trace {
    /// Validate an in-memory event stream (seq must start at 0 and
    /// increase by exactly 1).
    pub fn from_events(events: Vec<Event>) -> Result<Self, ReadError> {
        let mut prev_seq: Option<u64> = None;
        for (idx, evt) in events.iter().enumerate() {
            let expected = prev_seq.map(|p| p + 1).unwrap_or(0);
            if evt.seq != expected {
                return Err(ReadError::SeqNotMonotonic {
                    line: idx + 1,
                    prev: prev_seq,
                    expected,
                    got: evt.seq,
                });
            }
            prev_seq = Some(evt.seq);
        }
        Ok(Self { events })
    }

    /// Load and validate a run's stream from the store.
    pub async fn from_store(store: &dyn Store, run_id: &str) -> Result<Self, ReadError> {
        let events = store.run_events(run_id).await?;
        Self::from_events(events)
    }

    /// Parse and validate a stream from wire-format JSONL text (one
    /// event JSON object per line) — the export-bundle read path.
    pub fn from_jsonl(text: &str) -> Result<Self, ReadError> {
        let mut events = Vec::new();
        for (idx, line) in text.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let evt: Event = serde_json::from_str(line).map_err(|e| ReadError::Parse {
                line: idx + 1,
                source: e,
            })?;
            events.push(evt);
        }
        Self::from_events(events)
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn into_events(self) -> Vec<Event> {
        self.events
    }
}
