//! Continuously drained, bounded stream records; masking precedes retention.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use duhem_evidence::SecretRegistry;
use regex::Regex;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};

pub(super) const CAP: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Record {
    pub stream: &'static str,
    pub sequence: u64,
    pub elapsed_us: u64,
    pub generation: u64,
    pub text: String,
}

#[derive(Debug, Default)]
pub(super) struct Logs {
    records: VecDeque<Record>,
    bytes: [usize; 2],
    counts: [usize; 2],
    sequence: u64,
    dropped_through: u64,
    dropped_bytes: u64,
    pub ready: bool,
    pub errors: Vec<String>,
}

impl Logs {
    fn retain(
        &mut self,
        stream: &'static str,
        generation: u64,
        started: Instant,
        mut text: String,
    ) {
        if text.is_empty() {
            return;
        }
        let index = usize::from(stream == "stderr");
        self.sequence += 1;
        if text.len() > CAP {
            let mut drop = text.len() - CAP;
            while !text.is_char_boundary(drop) {
                drop += 1;
            }
            self.dropped_bytes += drop as u64;
            self.dropped_through = self.sequence;
            text.drain(..drop);
        }
        while self.bytes[index] + text.len() > CAP || self.counts[index] >= 1024 {
            let position = self
                .records
                .iter()
                .position(|record| record.stream == stream)
                .unwrap();
            let record = self.records.remove(position).unwrap();
            self.bytes[index] -= record.text.len();
            self.counts[index] -= 1;
            self.dropped_bytes += record.text.len() as u64;
            self.dropped_through = self.dropped_through.max(record.sequence);
        }
        self.bytes[index] += text.len();
        self.counts[index] += 1;
        self.records.push_back(Record {
            stream,
            sequence: self.sequence,
            elapsed_us: started.elapsed().as_micros() as u64,
            generation,
            text,
        });
    }

    pub fn snapshot(&self, cursor: u64) -> serde_json::Value {
        serde_json::json!({
            "logs": self.records.iter().filter(|record| record.sequence > cursor).collect::<Vec<_>>(),
            "next_cursor": self.sequence,
            "truncated": cursor < self.dropped_through,
            "dropped_bytes": self.dropped_bytes,
            "stream_errors": self.errors,
        })
    }

    pub fn cursor(&self) -> u64 {
        self.sequence
    }
}

pub(super) struct Drain {
    pub logs: Arc<Mutex<Logs>>,
    pub secrets: Arc<RwLock<SecretRegistry>>,
    pub stream: &'static str,
    pub generation: u64,
    pub started: Instant,
    pub probe: Option<Regex>,
}

pub(super) async fn drain(reader: impl AsyncRead + Unpin, config: Drain) {
    let mut reader = reader;
    let mut chunk = [0u8; 4096];
    let mut pending = Vec::new();
    let mut probe_window = Vec::new();
    loop {
        let count = match reader.read(&mut chunk).await {
            Ok(count) => count,
            Err(error) => {
                config
                    .logs
                    .lock()
                    .unwrap()
                    .errors
                    .push(format!("{}: {error}", config.stream));
                0
            }
        };
        pending.extend_from_slice(&chunk[..count]);
        let eof = count == 0;
        // Preserve incomplete UTF-8 across reads before applying the registry.
        let valid = match std::str::from_utf8(&pending) {
            Ok(_) => pending.len(),
            Err(error) if error.error_len().is_none() && !eof => error.valid_up_to(),
            Err(_) => pending.len(),
        };
        let text = String::from_utf8_lossy(&pending[..valid]);
        if let Some(probe) = &config.probe {
            // Only fresh bytes enter the transient readiness window. It is
            // independent of the secret matcher's unconsumed suffix.
            probe_window.extend_from_slice(&chunk[..count]);
            if probe_window.len() > CAP {
                let cut = probe_window.len() - CAP;
                probe_window.drain(..cut);
            }
            if probe.is_match(&String::from_utf8_lossy(&probe_window)) {
                config.logs.lock().unwrap().ready = true;
            }
        }
        let (masked, consumed) = config
            .secrets
            .read()
            .unwrap()
            .mask_stream_prefix(&text, eof);
        // For invalid UTF-8 the lossy representation changes byte offsets.
        // Hold its unconsumed suffix in normalized UTF-8 for the next read.
        let mut remaining = text[consumed..].as_bytes().to_vec();
        remaining.extend_from_slice(&pending[valid..]);
        pending = remaining;
        config.logs.lock().unwrap().retain(
            config.stream,
            config.generation,
            config.started,
            masked,
        );
        if eof {
            break;
        }
    }
}
