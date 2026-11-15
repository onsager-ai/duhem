//! CTRF ingestion pinned to v0.2.0:
//! https://github.com/ctrf-io/ctrf/blob/v0.2.0/schema/ctrf.schema.json
//! Counts come from tests, not producer summary totals. Unused metadata is ignored.
use serde::Deserialize;

use super::native_junit::{TestResult, TestStatus};

const SPEC_VERSION: &str = "0.2.0";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    spec_version: Option<String>,
    report_format: Option<String>,
    results: Results,
}

#[derive(Deserialize)]
struct Results {
    tests: Vec<Test>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Test {
    name: String,
    status: Status,
    duration: u64,
    message: Option<String>,
    trace: Option<String>,
    file_path: Option<String>,
    suite: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Passed,
    Failed,
    Skipped,
    Pending,
    Other,
}

pub(super) fn parse(native_id: &str, json: &str) -> Result<Vec<TestResult>, String> {
    let report: Report =
        serde_json::from_str(json).map_err(|error| format!("invalid CTRF report: {error}"))?;
    if let Some(version) = report.spec_version {
        let parts = version.split('.').collect::<Vec<_>>();
        if parts.len() != 3
            || parts
                .iter()
                .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
        {
            return Err(format!(
                "invalid CTRF specVersion `{version}`; pinned to {SPEC_VERSION}"
            ));
        }
        if parts[0]
            .parse::<u64>()
            .map_err(|_| "invalid CTRF major version")?
            > 0
        {
            return Err(format!(
                "CTRF specVersion `{version}` is newer than pinned major 0 ({SPEC_VERSION})"
            ));
        }
    }
    if report
        .report_format
        .as_deref()
        .is_some_and(|format| format != "CTRF")
    {
        return Err("CTRF reportFormat must be CTRF".into());
    }
    report
        .results
        .tests
        .into_iter()
        .map(|test| {
            if test.name.is_empty() || test.suite.as_ref().is_some_and(Vec::is_empty) {
                return Err(
                    "CTRF tests need a nonempty name and, when present, suite hierarchy".into(),
                );
            }
            // JSON array encoding preserves hierarchy without delimiter collisions.
            let classname = test
                .suite
                .map(|suite| serde_json::to_string(&suite).expect("suite serializes"));
            let test_id = serde_json::to_string(&(native_id, classname.as_deref(), &test.name))
                .expect("test identity serializes");
            let message = match (test.message, test.trace) {
                (Some(message), Some(trace)) => Some(format!("{message}\n{trace}")),
                (message, trace) => message.or(trace),
            };
            Ok(TestResult {
                test_id,
                classname,
                name: test.name,
                file: test.file_path,
                status: match test.status {
                    Status::Passed => TestStatus::Pass,
                    Status::Failed => TestStatus::Fail,
                    Status::Skipped | Status::Pending => TestStatus::Skip,
                    Status::Other => TestStatus::Error,
                },
                duration_ms: test.duration,
                message,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("../../tests/fixtures/ctrf/results.json");

    #[test]
    fn normalizes_published_statuses_and_identity() {
        let tests = parse("unit", FIXTURE).unwrap();
        assert_eq!(
            tests.iter().map(|t| t.status).collect::<Vec<_>>(),
            [
                TestStatus::Pass,
                TestStatus::Fail,
                TestStatus::Skip,
                TestStatus::Skip,
                TestStatus::Error
            ]
        );
        assert_eq!(tests[0].duration_ms, 125);
        assert_eq!(tests[0].file.as_deref(), Some("tests/example.js"));
        assert_eq!(parse("unit", FIXTURE).unwrap(), tests);
        assert_ne!(
            parse("other", FIXTURE).unwrap()[0].test_id,
            tests[0].test_id
        );
        assert!(parse("unit", &FIXTURE.replace("\"specVersion\": \"0.2.0\",", "")).is_ok());
    }

    #[test]
    fn rejects_invalid_reports_and_future_major() {
        for json in [
            "{",
            "{}",
            "{\"results\":{}}",
            "{\"results\":{\"tests\":[{}]}}",
        ] {
            assert!(parse("unit", json).is_err(), "{json}");
        }
        for (old, new) in [
            ("passed", "unknown"),
            ("\"duration\": 125", "\"duration\": -1"),
            ("\"duration\": 125,", ""),
            ("\"name\": \"passes\",", ""),
            ("0.2.0", "invalid"),
        ] {
            assert!(parse("unit", &FIXTURE.replace(old, new)).is_err());
        }
        assert!(
            parse("unit", &FIXTURE.replace("0.2.0", "1.0.0"))
                .unwrap_err()
                .contains("newer than pinned major")
        );
    }
}
