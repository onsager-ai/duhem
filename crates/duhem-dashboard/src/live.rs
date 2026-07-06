//! Live run streaming (#84): tail the store's event stream, push each
//! appended event to the browser over SSE.
//!
//! The load-bearing decision from the spec: the dashboard tails the
//! *evidence* (now the store's `events` rows), it does not open a
//! channel into the runtime. Rows are committed whole, so a reader
//! that consumes by `seq > last` never sees a half record — the
//! store-era equivalent of the old line-atomicity guarantee.
//!
//! Alignment choices (spec #84 "Human decides", resolved on the
//! conservative side):
//! - follow mechanism: bounded poll (250 ms), no notification channel;
//! - max stream lifetime: 1 hour, after which the stream closes with
//!   a `timeout` SSE event (a reconnecting client resumes via the
//!   replay-then-follow contract);
//! - backpressure: none beyond axum's own send buffering — events are
//!   read from the store on demand, so a slow client throttles its
//!   own reads instead of growing a queue.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::Event as SseEvent;
use duhem_evidence::{EventPayload, Store};
use futures::Stream;
use tokio::time::Instant;

pub const POLL_INTERVAL: Duration = Duration::from_millis(250);
pub const MAX_STREAM_LIFETIME: Duration = Duration::from_secs(60 * 60);

struct Follow {
    store: Arc<dyn Store>,
    run_id: String,
    /// Highest `seq` already consumed; the next poll asks the store
    /// for `seq > last_seq` only.
    last_seq: i64,
    pending: VecDeque<String>,
    /// Set once a `run_finished` event is queued: flush what's
    /// pending, then close.
    done: bool,
    deadline: Instant,
}

impl Follow {
    /// Pull newly-appended events into `pending` as wire-format JSON
    /// lines.
    async fn read_new_events(&mut self) -> Result<(), duhem_evidence::StoreError> {
        let events = self.store.events_after(&self.run_id, self.last_seq).await?;
        for evt in events {
            self.last_seq = evt.seq as i64;
            if matches!(evt.payload, EventPayload::RunFinished { .. }) {
                self.done = true;
            }
            if let Ok(line) = serde_json::to_string(&evt) {
                self.pending.push_back(line);
            }
        }
        Ok(())
    }
}

/// Replay-then-follow SSE stream over a run's event stream.
///
/// On connect, every event already in the store is sent (replay),
/// then new events are streamed as they are appended (follow). Each
/// event becomes one `trace` SSE event whose data is the wire-format
/// JSON line. The stream ends after `run_finished`, or with a
/// `timeout` event at the lifetime cap. Dropping the consumer (client
/// disconnect) drops the stream — no long-lived task survives the
/// connection.
pub fn live_stream(
    store: Arc<dyn Store>,
    run_id: String,
) -> impl Stream<Item = Result<SseEvent, Infallible>> {
    let follow = Follow {
        store,
        run_id,
        last_seq: -1,
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
            if let Err(e) = follow.read_new_events().await {
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
