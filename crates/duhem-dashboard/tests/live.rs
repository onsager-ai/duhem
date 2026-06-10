//! #84 live-streaming tests: replay-then-follow over a growing
//! `trace.jsonl`, partial-write safety, and the SSE route.

mod common;

use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use duhem_dashboard::live::live_stream;
use duhem_dashboard::{EvidenceReader, router};
use duhem_evidence::{EventPayload, EvidenceWriter, VerdictState, run_started};
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
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000A");
    let mut w = EvidenceWriter::new(&run_dir, "verifications/live.yml").unwrap();
    w.append(run_started("verifications/live.yml", BTreeMap::new()))
        .unwrap();

    let mut stream = Box::pin(live_stream(run_dir.clone()));

    // Replay: the event already on disk arrives first.
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
    .unwrap();
    w.append(EventPayload::RunFinished {
        verdict: VerdictState::Pass,
    })
    .unwrap();
    w.finish().unwrap();

    let rest = drain(stream.as_mut()).await;
    assert_eq!(rest.len(), 2, "got: {rest:#?}");
    assert!(rest[0].contains("check_finished"));
    assert!(rest[1].contains("run_finished"));
}

#[tokio::test]
async fn late_connect_replays_full_trace_with_no_gap_or_dupe() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000B");
    common::write_passing_run(&run_dir, "verifications/late.yml");
    let line_count = std::fs::read_to_string(run_dir.join("trace.jsonl"))
        .unwrap()
        .lines()
        .count();

    let events = drain(Box::pin(live_stream(run_dir))).await;
    assert_eq!(events.len(), line_count, "one SSE event per trace line");
    assert!(events.last().unwrap().contains("run_finished"));
    // seq values must be the trace's own, in order, exactly once.
    for (i, evt) in events.iter().enumerate() {
        assert!(
            evt.contains(&format!("\\\"seq\\\":{i},")),
            "event {i} out of order or duplicated: {evt}"
        );
    }
}

#[tokio::test]
async fn partial_line_is_never_delivered_half_parsed() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000C");
    let mut w = EvidenceWriter::new(&run_dir, "verifications/partial.yml").unwrap();
    w.append(run_started("verifications/partial.yml", BTreeMap::new()))
        .unwrap();
    w.finish().unwrap();

    let mut stream = Box::pin(live_stream(run_dir.clone()));
    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("replay event")
        .unwrap()
        .unwrap();
    assert!(format!("{first:?}").contains("run_started"));

    // Append half a line: nothing must come out of the stream.
    let trace_path = run_dir.join("trace.jsonl");
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&trace_path)
        .unwrap();
    f.write_all(br#"{"seq":1,"ts":"2026-06-10T00:00:00.000Z","kind":"run_fin"#)
        .unwrap();
    f.flush().unwrap();

    let nothing = tokio::time::timeout(Duration::from_millis(700), stream.next()).await;
    assert!(
        nothing.is_err(),
        "half-written line must not be delivered: {nothing:?}"
    );

    // Complete the line: it now arrives whole, and ends the stream.
    f.write_all(b"ished\",\"verdict\":\"pass\"}\n").unwrap();
    f.flush().unwrap();
    let rest = drain(stream.as_mut()).await;
    assert_eq!(rest.len(), 1, "got: {rest:#?}");
    assert!(rest[0].contains("run_finished"));
}

#[tokio::test]
async fn live_route_serves_sse_and_404s_unknown_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("01J0000000000000000000000D");
    common::write_passing_run(&run_dir, "verifications/sse.yml");
    let reader = EvidenceReader::new(tmp.path());

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
