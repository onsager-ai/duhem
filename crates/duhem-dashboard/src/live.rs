//! Live run streaming (#84): tail `trace.jsonl`, push each appended
//! event to the browser over SSE.
//!
//! The load-bearing decision from the spec: the dashboard tails the
//! evidence file, it does not open a channel into the runtime. The
//! writer's line-atomicity guarantee (whole line per `write`, `\n`
//! terminated) means a reader that only consumes complete lines never
//! sees a half record.
//!
//! Alignment choices (spec #84 "Human decides", resolved here on the
//! conservative side):
//! - follow mechanism: bounded poll (250 ms), no inotify dependency;
//! - max stream lifetime: 1 hour, after which the stream closes with
//!   a `timeout` SSE event (a reconnecting client resumes via the
//!   replay-then-follow contract);
//! - backpressure: none beyond axum's own send buffering — events are
//!   read from disk on demand, so a slow client throttles its own
//!   reads instead of growing a queue.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;

use axum::response::sse::Event as SseEvent;
use futures::Stream;
use tokio::time::Instant;

pub const POLL_INTERVAL: Duration = Duration::from_millis(250);
pub const MAX_STREAM_LIFETIME: Duration = Duration::from_secs(60 * 60);

struct Follow {
    path: PathBuf,
    /// Byte offset of the first unconsumed byte in `trace.jsonl`.
    /// Only ever advanced past a `\n`, so a partially-appended final
    /// line is re-read on the next poll instead of half-parsed.
    offset: u64,
    pending: VecDeque<String>,
    /// Set once a `run_finished` line is queued: flush what's pending,
    /// then close.
    done: bool,
    deadline: Instant,
}

impl Follow {
    /// Pull newly-appended complete lines into `pending`. Synchronous
    /// file I/O — call via [`Self::read_new_lines_blocking`] from
    /// async context.
    fn read_new_lines(&mut self) -> std::io::Result<()> {
        let mut f = std::fs::File::open(&self.path)?;
        f.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else {
            return Ok(());
        };
        for line in buf[..=last_nl].split(|&b| b == b'\n') {
            if line.is_empty() {
                continue;
            }
            let text = String::from_utf8_lossy(line).into_owned();
            if is_run_finished(&text) {
                self.done = true;
            }
            self.pending.push_back(text);
        }
        self.offset += (last_nl + 1) as u64;
        Ok(())
    }

    /// Run [`Self::read_new_lines`] on the blocking pool so the file
    /// I/O never stalls a Tokio worker (review note on PR #88).
    async fn read_new_lines_blocking(mut self) -> (Self, std::io::Result<()>) {
        tokio::task::spawn_blocking(move || {
            let result = self.read_new_lines();
            (self, result)
        })
        .await
        .expect("trace follow-reader task panicked")
    }
}

fn is_run_finished(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| {
            v.get("kind")
                .and_then(|k| k.as_str())
                .map(|k| k == "run_finished")
        })
        .unwrap_or(false)
}

/// Replay-then-follow SSE stream over `<run_dir>/trace.jsonl`.
///
/// On connect, every line already on disk is sent (replay), then new
/// lines are streamed as they are appended (follow). Each trace line
/// becomes one `trace` SSE event whose data is the raw JSON line. The
/// stream ends after `run_finished`, or with a `timeout` event at the
/// lifetime cap. Dropping the consumer (client disconnect) drops the
/// stream and the file handle with it — no long-lived task survives
/// the connection (each poll's read is a one-shot `spawn_blocking`).
pub fn live_stream(run_dir: PathBuf) -> impl Stream<Item = Result<SseEvent, Infallible>> {
    let follow = Follow {
        path: run_dir.join("trace.jsonl"),
        offset: 0,
        pending: VecDeque::new(),
        done: false,
        deadline: Instant::now() + MAX_STREAM_LIFETIME,
    };

    futures::stream::unfold(Some(follow), |state| async move {
        let mut follow = state?;
        loop {
            if let Some(line) = follow.pending.pop_front() {
                let evt = SseEvent::default().event("trace").data(line);
                let done_after = follow.done && follow.pending.is_empty();
                return Some((Ok(evt), (!done_after).then_some(follow)));
            }
            if follow.done {
                return None;
            }
            let (returned, result) = follow.read_new_lines_blocking().await;
            follow = returned;
            if let Err(e) = result {
                let evt = SseEvent::default().event("error").data(e.to_string());
                return Some((Ok(evt), None));
            }
            if follow.pending.is_empty() {
                if Instant::now() >= follow.deadline {
                    let evt = SseEvent::default()
                        .event("timeout")
                        .data("live stream lifetime cap reached; reconnect to resume");
                    return Some((Ok(evt), None));
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::is_run_finished;

    #[test]
    fn run_finished_detection_is_structural_not_substring() {
        assert!(is_run_finished(
            r#"{"seq":9,"ts":"2026-05-08T12:00:00.000Z","kind":"run_finished","verdict":"pass"}"#
        ));
        // A check observation *mentioning* run_finished must not end
        // the stream.
        assert!(!is_run_finished(
            r#"{"seq":3,"kind":"step_observation","output_name":"body","value":"kind run_finished"}"#
        ));
        assert!(!is_run_finished("not json"));
    }
}
