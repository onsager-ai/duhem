//! JUnit ingestion counts testcase records, never producer-supplied suite totals.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TestStatus {
    Pass,
    Fail,
    Skip,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TestResult {
    pub test_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classname: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub status: TestStatus,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub(super) fn parse(native_id: &str, xml: &str) -> Result<Vec<TestResult>, String> {
    // DTDs/entities are deliberately disabled by roxmltree's default options.
    let doc = roxmltree::Document::parse(xml).map_err(|error| error.to_string())?;
    let root = doc.root_element();
    if !matches!(root.tag_name().name(), "testsuite" | "testsuites") {
        return Err("expected a testsuite or testsuites root".into());
    }
    root.descendants()
        .filter(|node| node.has_tag_name("testcase"))
        .map(|case| {
            let name = case
                .attribute("name")
                .ok_or("testcase needs a name")?
                .to_string();
            let classname = case.attribute("classname").map(str::to_string);
            // JSON tuple encoding avoids delimiter collisions and stays stable across runs.
            let test_id = serde_json::to_string(&(native_id, classname.as_deref(), &name))
                .expect("test identity serializes");
            let seconds = case
                .attribute("time")
                .unwrap_or("0")
                .parse::<f64>()
                .map_err(|error| format!("invalid testcase time: {error}"))?;
            if !seconds.is_finite() || seconds < 0.0 || seconds * 1000.0 > u64::MAX as f64 {
                return Err("testcase time must be finite, nonnegative milliseconds".into());
            }
            let child = |tag| case.children().find(|node| node.has_tag_name(tag));
            let (status, diagnostic) = if let Some(node) = child("error") {
                (TestStatus::Error, Some(node))
            } else if let Some(node) = child("failure") {
                (TestStatus::Fail, Some(node))
            } else if let Some(node) = child("skipped") {
                (TestStatus::Skip, Some(node))
            } else {
                (TestStatus::Pass, None)
            };
            let message = diagnostic.and_then(|node| {
                let body: String = node
                    .descendants()
                    .filter(|node| node.is_text())
                    .filter_map(|node| node.text())
                    .collect();
                let attribute = node.attribute("message").unwrap_or("");
                let text = match (attribute.is_empty(), body.trim().is_empty()) {
                    (false, false) if attribute != body.trim() => {
                        format!("{attribute}\n{}", body.trim())
                    }
                    (false, _) => attribute.to_string(),
                    _ => body.trim().to_string(),
                };
                (!text.is_empty()).then_some(text)
            });
            Ok(TestResult {
                test_id,
                classname,
                name,
                file: case.attribute("file").map(str::to_string),
                status,
                duration_ms: (seconds * 1000.0).round() as u64,
                message,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn producer_fixtures_and_nested_suites() {
        for (xml, counts) in [
            (
                include_str!("../../tests/fixtures/junit/nextest.xml"),
                [1, 1, 0, 1],
            ),
            (
                include_str!("../../tests/fixtures/junit/pytest.xml"),
                [1, 1, 1, 1],
            ),
            (
                include_str!("../../tests/fixtures/junit/jest-junit.xml"),
                [1, 1, 0, 1],
            ),
            (
                include_str!("../../tests/fixtures/junit/playwright.xml"),
                [1, 1, 0, 1],
            ),
            (
                include_str!("../../tests/fixtures/junit/go-junit-report.xml"),
                [1, 1, 0, 1],
            ),
        ] {
            let parsed = parse("unit", xml).unwrap();
            let actual = [
                TestStatus::Pass,
                TestStatus::Fail,
                TestStatus::Error,
                TestStatus::Skip,
            ]
            .map(|status| parsed.iter().filter(|test| test.status == status).count());
            assert_eq!(actual, counts);
            assert_eq!(parsed[0].duration_ms, 125);
            assert_eq!(parsed, parse("unit", xml).unwrap());
            assert_ne!(parsed[0].test_id, parse("other", xml).unwrap()[0].test_id);
        }
        let nested = parse("n", "<testsuites><testsuite><testsuite><testcase name='nested'/></testsuite></testsuite></testsuites>").unwrap();
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].duration_ms, 0);
    }

    #[test]
    fn rejects_invalid_reports_and_decodes_messages() {
        for xml in [
            "",
            "<testsuite>",
            "<html/>",
            "<testsuite><testcase/></testsuite>",
            "<testsuite><testcase name='n' time='NaN'/></testsuite>",
        ] {
            assert!(parse("n", xml).is_err(), "{xml}");
        }
        let result = parse("n", "<testsuite><testcase name='n'><failure message='a &amp; b'><![CDATA[trace <here>]]></failure></testcase></testsuite>").unwrap();
        assert_eq!(result[0].message.as_deref(), Some("a & b\ntrace <here>"));
    }
}
