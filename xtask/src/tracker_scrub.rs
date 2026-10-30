//! Scheduled detection of denied vocabulary on configured authors' artifacts.
//! Read-only GitHub API calls; never writes tracker content or logs match text.
use crate::private_vocab::scan_line;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

const REPO: &str = "onsager-ai/duhem";
const BOUND: &str = "Configured authors only: non-configured authors and other trackers are excluded. This detects after publication; it cannot prevent initial exposure. Detection only: no automatic edits. Match text is omitted from logs.";

#[derive(Debug, PartialEq, Eq)]
struct Hit {
    url: String,
    field: &'static str,
    line: usize,
}

pub fn run(args: Vec<String>) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")?;
    let mut authors_file = root.join(".github/tracker-scrub-authors.json");
    let mut fixture = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--authors-file" => {
                authors_file = PathBuf::from(args.next().context("--authors-file needs PATH")?)
            }
            "--fixture" => {
                fixture = Some(PathBuf::from(args.next().context("--fixture needs PATH")?))
            }
            "--help" => {
                println!("tracker-scrub [--authors-file PATH] [--fixture PATH]\n{BOUND}");
                return Ok(());
            }
            _ => bail!("unknown tracker-scrub option: {arg}"),
        }
    }
    let authors = read_authors(&authors_file)?;
    println!(
        "tracker-scrub: repository {REPO}; authors: {}",
        authors.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    println!("{BOUND}");
    let mut hits = Vec::new();
    let mut inspected = 0;
    if let Some(path) = fixture {
        let records: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).context("read fixture")?)
                .context("parse fixture")?;
        scan_records(&records, &authors, &mut inspected, &mut hits)?;
    } else {
        // The issues endpoint includes PR bodies as well. Reviews and inline
        // comments are separate APIs and must not be lost at that boundary.
        let issues = fetch_pages("issues?state=all&per_page=100")?;
        for page in &issues {
            scan_records(page, &authors, &mut inspected, &mut hits)?;
        }
        for endpoint in [
            "issues/comments?per_page=100",
            "pulls/comments?per_page=100",
        ] {
            for page in fetch_pages(endpoint)? {
                scan_records(&page, &authors, &mut inspected, &mut hits)?;
            }
        }
        for issue in issues
            .iter()
            .flat_map(|page| page.as_array().into_iter().flatten())
        {
            if issue.get("pull_request").is_some() {
                let number = issue["number"].as_u64().context("PR number missing")?;
                for page in fetch_pages(&format!("pulls/{number}/reviews?per_page=100"))? {
                    scan_records(&page, &authors, &mut inspected, &mut hits)?;
                }
            }
        }
    }
    for hit in &hits {
        println!("  {} ({}:{})", hit.url, hit.field, hit.line);
    }
    println!(
        "tracker-scrub: inspected {inspected} configured-author artifacts; {} matching locations",
        hits.len()
    );
    if !hits.is_empty() {
        bail!(
            "{} matching locations; review the linked artifacts (content omitted)",
            hits.len()
        );
    }
    Ok(())
}

fn read_authors(path: &Path) -> Result<BTreeSet<String>> {
    let values: Vec<String> = serde_json::from_str(
        &std::fs::read_to_string(path)
            .with_context(|| format!("read author configuration {}", path.display()))?,
    )
    .context("author configuration must be a JSON array of login strings")?;
    let authors: BTreeSet<_> = values
        .into_iter()
        .map(|author| author.trim().to_ascii_lowercase())
        .collect();
    if authors.is_empty() || authors.contains("") {
        bail!("configure at least one non-empty author login explicitly");
    }
    Ok(authors)
}

fn fetch_pages(endpoint: &str) -> Result<Vec<Value>> {
    let response = Command::new("gh")
        .args([
            "api",
            "--paginate",
            "--slurp",
            &format!("repos/{REPO}/{endpoint}"),
        ])
        .output()
        .context("run gh api")?;
    if !response.status.success() {
        bail!(
            "GitHub read failed for {endpoint}: {}",
            String::from_utf8_lossy(&response.stderr)
        );
    }
    serde_json::from_slice(&response.stdout).context("parse paginated GitHub response")
}

fn scan_records(
    records: &Value,
    authors: &BTreeSet<String>,
    inspected: &mut usize,
    hits: &mut Vec<Hit>,
) -> Result<()> {
    for record in records.as_array().context("expected an artifact array")? {
        let Some(author) = record["user"]["login"].as_str() else {
            continue;
        };
        if !authors.contains(&author.to_ascii_lowercase()) {
            continue;
        }
        *inspected += 1;
        let url = record["html_url"]
            .as_str()
            .context("artifact URL missing")?;
        for field in ["title", "body"] {
            if let Some(text) = record[field].as_str() {
                for (line, text) in text.lines().enumerate() {
                    if !scan_line(text).is_empty() {
                        hits.push(Hit {
                            url: url.into(),
                            field,
                            line: line + 1,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn fail_first_fixture_is_detected_at_its_location_without_reprinting_content() {
        let records: Value =
            serde_json::from_str(include_str!("../fixtures/tracker-scrub.json")).unwrap();
        let mut hits = Vec::new();
        let mut inspected = 0;
        scan_records(
            &records,
            &BTreeSet::from(["configured".into()]),
            &mut inspected,
            &mut hits,
        )
        .unwrap();
        assert_eq!(inspected, 2);
        assert_eq!(
            hits,
            vec![Hit {
                url: "https://github.com/onsager-ai/duhem/issues/1#issuecomment-1".into(),
                field: "body",
                line: 2
            }]
        );
        assert!(!format!("{hits:?}").contains("dogfood"));
    }
    #[test]
    fn title_review_and_inline_comment_shapes_share_the_same_matcher() {
        let records = json!([
            {"user":{"login":"configured"},"html_url":"title", "title":"dogfood"},
            {"user":{"login":"CONFIGURED"},"html_url":"review", "body":"seam"},
            {"user":{"login":"configured"},"html_url":"inline", "body":"onsager-ai/duhem is on GitHub"},
            {"user":{"login":"third-party"},"html_url":"historical", "body":"dogfood"}
        ]);
        let mut hits = Vec::new();
        let mut inspected = 0;
        scan_records(
            &records,
            &BTreeSet::from(["configured".into()]),
            &mut inspected,
            &mut hits,
        )
        .unwrap();
        assert_eq!(inspected, 3);
        assert_eq!(
            hits.iter().map(|hit| hit.url.as_str()).collect::<Vec<_>>(),
            vec!["title", "review"]
        );
    }
    #[test]
    fn malformed_api_payloads_do_not_report_a_clean_scan() {
        assert!(scan_records(&Value::Null, &BTreeSet::new(), &mut 0, &mut Vec::new()).is_err());
    }
}
