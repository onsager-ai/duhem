//! Report evidence and bounded failure navigation for native runs.
use std::path::Path;

use duhem_evidence::EvidenceWriter;
use duhem_judge::{InconclusiveCause, VerdictState};
use duhem_schema::NativeReportFormat;
use duhem_summary::{NativeRunSummary, NativeTestCounts, NativeTestFailure};

use super::EngineError;
use super::native_ctrf;
use super::native_junit::{self, TestStatus};

const FAILURE_LIMIT: usize = 10;
const MESSAGE_LIMIT: usize = 2048;

pub(super) async fn collect(
    writer: &mut EvidenceWriter,
    native_id: &str,
    path: &Path,
    format: NativeReportFormat,
    result: &mut NativeRunSummary,
) -> Result<(), EngineError> {
    let label = match format {
        NativeReportFormat::Junit => "JUnit",
        NativeReportFormat::Ctrf => "CTRF",
    };
    let parsed = match tokio::fs::read(path).await {
        Ok(bytes) => {
            // Decode before storage: the writer masks UTF-8 text, while arbitrary
            // binary evidence is byte-preserving. Both formats require UTF-8.
            match String::from_utf8(bytes) {
                Ok(text) => {
                    result.report_blob_sha256 = Some(writer.write_blob(text.as_bytes()).await?.0);
                    match format {
                        NativeReportFormat::Junit => native_junit::parse(native_id, &text),
                        NativeReportFormat::Ctrf => native_ctrf::parse(native_id, &text),
                    }
                }
                Err(_) => Err(format!("{label} report must be UTF-8 text")),
            }
        }
        Err(error) => Err(format!("cannot read {label} report: {error}")),
    };
    let mut tests = match parsed {
        Ok(tests) => tests,
        Err(error) => {
            result.outcome = VerdictState::Inconclusive(InconclusiveCause::MissingObservation);
            // Keep the exit-code/output detail: a missing report is often a
            // build failure, and its output is the evidence that explains it.
            let error = writer.mask_text(&error);
            result.detail = Some(match result.detail.take() {
                Some(detail) => format!("{error}\n{detail}"),
                None => error,
            });
            return Ok(());
        }
    };
    let mut counts = NativeTestCounts::default();
    for test in &mut tests {
        test.name = writer.mask_text(&test.name);
        test.classname = test.classname.as_ref().map(|text| writer.mask_text(text));
        test.test_id = serde_json::to_string(&(
            writer.mask_text(native_id),
            test.classname.as_deref(),
            &test.name,
        ))
        .expect("masked test identity serializes");
        test.file = test.file.as_ref().map(|text| writer.mask_text(text));
        test.message = test.message.as_ref().map(|text| writer.mask_text(text));
        match test.status {
            TestStatus::Pass => counts.passed += 1,
            TestStatus::Fail => counts.failed += 1,
            TestStatus::Error => counts.errored += 1,
            TestStatus::Skip => counts.skipped += 1,
        }
        if matches!(test.status, TestStatus::Fail | TestStatus::Error)
            && result.failing_tests.len() < FAILURE_LIMIT
        {
            result.failing_tests.push(NativeTestFailure {
                test_id: test.test_id.clone(),
                name: test.name.clone(),
                file: test.file.clone(),
                message: test.message.as_deref().map(truncate_message),
            });
        }
    }
    result.tests = counts.passed + counts.failed + counts.errored + counts.skipped;
    if counts.failed + counts.errored > 0 {
        result.outcome = VerdictState::Fail;
    }
    result.counts = Some(counts);
    // Mask individual strings before JSON encoding, including secrets containing
    // quotes/newlines whose escaped representation would evade byte matching.
    let json = serde_json::to_vec(&tests).expect("normalized tests serialize");
    result.results_blob_sha256 = Some(writer.write_blob(&json).await?.0);
    Ok(())
}

fn truncate_message(message: &str) -> String {
    let mut chars = message.chars();
    let mut text: String = chars.by_ref().take(MESSAGE_LIMIT).collect();
    if chars.next().is_some() {
        text.push_str("… [truncated]");
    }
    text
}
