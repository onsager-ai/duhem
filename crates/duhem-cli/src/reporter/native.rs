//! Native command rendering and check durations for cross-layer totals.
use std::io::Write;

pub(super) fn write_native(
    out: &mut dyn Write,
    runs: &[duhem_summary::NativeRunSummary],
) -> std::io::Result<()> {
    if runs.is_empty() {
        return Ok(());
    }
    writeln!(out, "Native runs")?;
    for run in runs {
        writeln!(
            out,
            "  {}: {} ({}ms{})",
            run.id,
            run.outcome,
            run.duration_ms,
            if run.required { "" } else { ", diagnostic" }
        )?;
        if let Some(detail) = &run.detail {
            for line in detail.lines() {
                writeln!(out, "    {line}")?;
            }
        }
    }
    Ok(())
}

pub(super) fn native_check_duration(events: &[duhem_evidence::Event]) -> u64 {
    let mut start = None;
    let mut total = 0u64;
    for event in events {
        match &event.payload {
            duhem_evidence::EventPayload::StepStarted { .. } => start = Some(event.ts),
            duhem_evidence::EventPayload::StepFinished { .. } => {
                if let Some(started) = start.take() {
                    total += (event.ts - started).num_milliseconds().max(0) as u64;
                }
            }
            _ => {}
        }
    }
    total
}
