//! Reader for an on-disk run trace.
//!
//! Yields typed [`Event`] values, one per line. Untyped lines (a
//! `kind` not in the closed set) are a hard error — evidence is
//! structured by contract, and a tool that silently skips unknown
//! events would let the format rot under us.
//!
//! `seq` monotonicity is enforced on read (gap or backwards = hard
//! error). The writer guarantees this on write; the reader is the
//! second line of defense, especially against hand-edited or
//! out-of-tree-produced traces.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::event::Event;

#[derive(Debug, Error)]
pub enum ReadError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error on line {line}: {source}")]
    Parse {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("seq {got} on line {line} is not monotonic after seq {prev}")]
    SeqNotMonotonic { line: usize, prev: u64, got: u64 },
}

/// An open trace. The trace is fully materialized into memory on
/// `open`. Run traces are bounded by the number of steps in a
/// Verification Definition and assertions per check — small enough
/// that the simplicity of an in-memory `Vec` is worth more than the
/// streaming property at v1.
#[derive(Debug, Clone)]
pub struct Trace {
    run_dir: PathBuf,
    events: Vec<Event>,
}

impl Trace {
    /// Open and validate the trace at `<run_dir>/trace.jsonl`.
    pub fn open(run_dir: impl AsRef<Path>) -> Result<Self, ReadError> {
        let run_dir = run_dir.as_ref().to_path_buf();
        let trace_path = run_dir.join("trace.jsonl");
        let f = File::open(&trace_path)?;
        let reader = BufReader::new(f);

        let mut events = Vec::new();
        let mut prev_seq: Option<u64> = None;

        for (idx, line) in reader.lines().enumerate() {
            let line = line?;
            if line.is_empty() {
                continue;
            }
            let evt: Event = serde_json::from_str(&line).map_err(|e| ReadError::Parse {
                line: idx + 1,
                source: e,
            })?;
            let expected = prev_seq.map(|p| p + 1).unwrap_or(0);
            if evt.seq != expected {
                return Err(ReadError::SeqNotMonotonic {
                    line: idx + 1,
                    prev: prev_seq.unwrap_or(0),
                    got: evt.seq,
                });
            }
            prev_seq = Some(evt.seq);
            events.push(evt);
        }

        Ok(Self { run_dir, events })
    }

    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn into_events(self) -> Vec<Event> {
        self.events
    }

    /// Read a blob by its content address.
    pub fn read_blob(&self, sha256: &str) -> Result<Vec<u8>, ReadError> {
        let path = self.run_dir.join("blobs").join(sha256);
        Ok(std::fs::read(path)?)
    }
}
