//! #84 live-streaming tests: replay-then-follow over a growing event
//! stream in the store, and the SSE route.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use duhem_dashboard::live::live_stream;
use duhem_dashboard::{EvidenceReader, router};
use duhem_evidence::{EventPayload, EvidenceWriter, Store, VerdictState, run_started};
use futures::StreamExt;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Collect SSE events from the stream until it ends, with a guard
/// timeout so a regression can't hang the suite.
async fn drain<S>(mut stream: S) -> Vec<String>
where
    S: futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>> + Unpin,
{
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(10), stream.next()).await {
            Ok(Some(Ok(evt))) => out.push(format!("{evt:?}")),
            Ok(None) => return out,
            Ok(Some(Err(_))) => unreachable!("Infallible"),
            Err(_) => panic!("live stream did not terminate; got so far: {out:#?}"),
        }
    }
}

#[tokio::test]
async fn connect_mid_flight_streams_incrementally_and_ends_on_run_finished() {
    let (_tmp, rw, ro) = common::open_stores().await;
    let mut w = EvidenceWriter::begin(
        rw,
        "01J0000000000000000000000A",
        "verifications/live.yml",
        BTreeMap::new(),
    )
    .await
    .unwrap();
    w.append(run_started("verifications/live.yml", BTreeMap::new()))
        .await
        .unwrap();

    let mut stream = Box::pin(live_stream(
        ro.clone(),
        "01J0000000000000000000000A".to_string(),
    ));

    // Replay: the event already in the store arrives first.
    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("replay event")
        .unwrap()
        .unwrap();
    assert!(format!("{first:?}").contains("run_started"));

    // Follow: append while the stream is live; the new events arrive
    // without reconnecting, and run_finished terminates the stream.
    w.append(EventPayload::CheckFinished {
        check_id: "AC-1.1".into(),
        verdict: VerdictState::Pass,
    })
    .await
    .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Pass,
    })
    .await
    .unwrap();
    w.finish().await.unwrap();

    let rest = drain(stream.as_mut()).await;
    assert_eq!(rest.len(), 2, "got: {rest:#?}");
    assert!(rest[0].contains("check_finished"));
    assert!(rest[1].contains("run_finished"));
}

#[tokio::test]
async fn late_connect_replays_full_stream_with_no_gap_or_dupe() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(rw, "01J0000000000000000000000B", "verifications/late.yml").await;
    let event_count = ro
        .run_events("01J0000000000000000000000B")
        .await
        .unwrap()
        .len();

    let events = drain(Box::pin(live_stream(
        ro,
        "01J0000000000000000000000B".to_string(),
    )))
    .await;
    assert_eq!(events.len(), event_count, "one SSE event per stored event");
    assert!(events.last().unwrap().contains("run_finished"));
    // seq values must be the stream's own, in order, exactly once.
    for (i, evt) in events.iter().enumerate() {
        assert!(
            evt.contains(&format!("\\\"seq\\\":{i},")),
            "event {i} out of order or duplicated: {evt}"
        );
    }
}

#[tokio::test]
async fn live_route_serves_sse_and_404s_unknown_runs() {
    let (_tmp, rw, ro) = common::open_stores().await;
    common::write_passing_run(rw, "01J0000000000000000000000D", "verifications/sse.yml").await;
    let reader = EvidenceReader::new(ro);

    let res = router(reader.clone())
        .oneshot(
            Request::get("/api/runs/01J0000000000000000000000D/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), axum::http::StatusCode::OK);
    let content_type = res.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(content_type.starts_with("text/event-stream"));
    // Finished run: the stream replays to run_finished and closes, so
    // collecting the body terminates.
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("event: trace"));
    assert!(text.contains("run_finished"));

    let res = router(reader)
        .oneshot(
            Request::get("/api/runs/01J0000000000000000000000Z/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), axum::http::StatusCode::NOT_FOUND);
}
