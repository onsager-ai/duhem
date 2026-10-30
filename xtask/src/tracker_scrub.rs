//! Scheduled, read-only detection for configured authors after a date baseline.
use crate::private_vocab::{
    PRIVATE_NAMES, PrivateName, all_skill_rule_ids, scan_private_line, skill_rule_ids,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Utc};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

const REPO: &str = "onsager-ai/duhem";
const BOUND: &str = "Excludes non-configured authors, untouched posts before the configured updated-date baseline, and other trackers. This detects after publication; it cannot prevent initial exposure. Detection only: no automatic edits. Match text is omitted from logs.";

struct Config {
    authors: BTreeSet<String>,
    since: DateTime<Utc>,
}
#[derive(Debug, PartialEq, Eq)]
struct Hit {
    url: String,
    field: &'static str,
    line: usize,
}
#[derive(Default)]
struct Scan {
    inspected: usize,
    eligible: usize,
    hits: Vec<Hit>,
}
#[derive(Default)]
struct Measurement {
    skill_vocabulary: BTreeMap<String, BTreeMap<i32, usize>>,
    private_names: BTreeMap<String, BTreeMap<i32, usize>>,
    private_names_in_date_scope: BTreeMap<String, BTreeMap<i32, usize>>,
}

pub fn run(args: Vec<String>) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")?;
    let mut config_path = root.join(".github/tracker-scrub.json");
    let mut fixture = None;
    let mut measure = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config_path = PathBuf::from(args.next().context("--config needs PATH")?),
            "--fixture" => {
                fixture = Some(PathBuf::from(args.next().context("--fixture needs PATH")?))
            }
            "--measure" => measure = true,
            "--help" => {
                println!("tracker-scrub [--config PATH] [--fixture PATH] [--measure]\n{BOUND}");
                return Ok(());
            }
            _ => bail!("unknown tracker-scrub option: {arg}"),
        }
    }
    let config = read_config(&config_path)?;
    let mut scan = Scan::default();
    let mut counts = Measurement::default();
    for id in all_skill_rule_ids() {
        counts.skill_vocabulary.insert(id, BTreeMap::new());
    }
    for rule in PRIVATE_NAMES {
        counts.private_names.insert(rule.id.into(), BTreeMap::new());
        counts
            .private_names_in_date_scope
            .insert(rule.id.into(), BTreeMap::new());
    }
    let mut inspect = |records: &Value| {
        scan_records(
            records,
            &config,
            PRIVATE_NAMES,
            measure,
            &mut scan,
            &mut counts,
        )
    };
    if let Some(path) = fixture {
        let records = serde_json::from_str(&std::fs::read_to_string(path).context("read fixture")?)
            .context("parse fixture")?;
        inspect(&records)?;
    } else {
        let issues = fetch_pages("issues?state=all&per_page=100")?;
        for page in &issues {
            inspect(page)?;
        }
        for endpoint in [
            "issues/comments?per_page=100",
            "pulls/comments?per_page=100",
        ] {
            for page in fetch_pages(endpoint)? {
                inspect(&page)?;
            }
        }
        for issue in issues
            .iter()
            .flat_map(|page| page.as_array().into_iter().flatten())
        {
            if issue.get("pull_request").is_some() {
                let number = issue["number"].as_u64().context("PR number missing")?;
                for page in fetch_reviews(number)? {
                    inspect(&page)?;
                }
            }
        }
    }
    if measure {
        println!(
            "{}",
            json!({"repository":REPO, "configured_artifacts":scan.inspected,
            "in_date_scope_artifacts":scan.eligible, "updated_on_or_after":config.since.to_rfc3339(),
            "private_matching_locations_in_date_scope":scan.hits.len(),
            "year_basis":"updated_at UTC year; all history for configured authors; per-rule matching lines",
            "skill_vocabulary":counts.skill_vocabulary, "private_names":counts.private_names,
            "private_names_in_date_scope":counts.private_names_in_date_scope})
        );
        return Ok(());
    }
    println!(
        "tracker-scrub: repository {REPO}; authors: {}; updated on or after {}",
        config
            .authors
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        config.since.to_rfc3339()
    );
    println!("{BOUND}");
    for hit in &scan.hits {
        println!("  {} ({}:{})", hit.url, hit.field, hit.line);
    }
    println!(
        "tracker-scrub: {} configured-author artifacts; {} in date scope; {} matching locations",
        scan.inspected,
        scan.eligible,
        scan.hits.len()
    );
    if !scan.hits.is_empty() {
        bail!(
            "{} matching locations; review linked artifacts (content omitted)",
            scan.hits.len()
        );
    }
    Ok(())
}

fn read_config(path: &Path) -> Result<Config> {
    let value: Value = serde_json::from_str(
        &std::fs::read_to_string(path)
            .with_context(|| format!("read configuration {}", path.display()))?,
    )
    .context("parse configuration")?;
    let authors: BTreeSet<_> = value["authors"]
        .as_array()
        .context("configure authors explicitly")?
        .iter()
        .map(|author| {
            author
                .as_str()
                .map(|author| author.trim().to_ascii_lowercase())
                .context("author must be a login string")
        })
        .collect::<Result<_>>()?;
    if authors.is_empty() || authors.contains("") {
        bail!("configure at least one non-empty author login explicitly");
    }
    let since = DateTime::parse_from_rfc3339(
        value["updated_on_or_after"]
            .as_str()
            .context("configure updated_on_or_after explicitly")?,
    )
    .context("baseline must be an RFC3339 UTC timestamp")?
    .with_timezone(&Utc);
    Ok(Config { authors, since })
}

fn gh_pages(args: &[&str]) -> Result<Vec<Value>> {
    let response = Command::new("gh")
        .args(args)
        .output()
        .context("run read-only gh api")?;
    if !response.status.success() {
        bail!(
            "GitHub read failed: {}",
            String::from_utf8_lossy(&response.stderr)
        );
    }
    serde_json::from_slice(&response.stdout).context("parse paginated GitHub response")
}
fn fetch_pages(endpoint: &str) -> Result<Vec<Value>> {
    gh_pages(&[
        "api",
        "--paginate",
        "--slurp",
        &format!("repos/{REPO}/{endpoint}"),
    ])
}
fn fetch_reviews(number: u64) -> Result<Vec<Value>> {
    // REST reviews expose submitted_at, not their last edit time. GraphQL's
    // updatedAt is required so editing an old review re-enters the date scope.
    let query = r#"query($number:Int!,$endCursor:String){repository(owner:"onsager-ai",name:"duhem"){pullRequest(number:$number){reviews(first:100,after:$endCursor){nodes{author{login} body url createdAt updatedAt} pageInfo{hasNextPage endCursor}}}}}"#;
    gh_pages(&["api", "graphql", "--paginate", "--slurp", "-f", &format!("query={query}"), "-F", &format!("number={number}")])?.iter().map(|page| {
        let nodes = page.pointer("/data/repository/pullRequest/reviews/nodes").and_then(Value::as_array).context("review nodes missing")?;
        Ok(Value::Array(nodes.iter().map(|node| json!({"user":{"login":node["author"]["login"]}, "html_url":node["url"], "body":node["body"], "updated_at":node["updatedAt"]})).collect()))
    }).collect()
}

fn scan_records(
    records: &Value,
    config: &Config,
    rules: &[PrivateName],
    measure: bool,
    scan: &mut Scan,
    counts: &mut Measurement,
) -> Result<()> {
    for record in records.as_array().context("expected an artifact array")? {
        let Some(author) = record["user"]["login"].as_str() else {
            continue;
        };
        if !config.authors.contains(&author.to_ascii_lowercase()) {
            continue;
        }
        scan.inspected += 1;
        let updated = chrono::DateTime::parse_from_rfc3339(
            record["updated_at"]
                .as_str()
                .context("artifact updated_at missing")?,
        )
        .context("invalid artifact updated_at")?
        .with_timezone(&chrono::Utc);
        let eligible = updated >= config.since;
        if eligible {
            scan.eligible += 1;
        }
        if !measure && !eligible {
            continue;
        }
        for field in ["title", "body"] {
            if let Some(text) = record[field].as_str() {
                for (line, text) in text.lines().enumerate() {
                    let private = scan_private_line(text, rules);
                    if measure {
                        for id in skill_rule_ids(text) {
                            *counts
                                .skill_vocabulary
                                .entry(id)
                                .or_default()
                                .entry(updated.year())
                                .or_default() += 1;
                        }
                        for id in &private {
                            *counts
                                .private_names
                                .entry((*id).into())
                                .or_default()
                                .entry(updated.year())
                                .or_default() += 1;
                            if eligible {
                                *counts
                                    .private_names_in_date_scope
                                    .entry((*id).into())
                                    .or_default()
                                    .entry(updated.year())
                                    .or_default() += 1;
                            }
                        }
                    }
                    if eligible && !private.is_empty() {
                        let url = record["html_url"]
                            .as_str()
                            .context("artifact URL missing")?;
                        scan.hits.push(Hit {
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
    use sha2::{Digest, Sha256};
    fn policy() -> Vec<PrivateName> {
        vec![PrivateName {
            id: "test",
            length: 18,
            digest: Sha256::digest(b"restricted-example").into(),
        }]
    }
    fn config() -> Config {
        Config {
            authors: BTreeSet::from(["configured".into()]),
            since: DateTime::parse_from_rfc3339("2026-10-30T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }
    #[test]
    fn old_untouched_hit_is_ignored_but_old_edited_and_new_hits_are_reported() {
        let records: Value =
            serde_json::from_str(include_str!("../fixtures/tracker-scrub.json")).unwrap();
        let mut scan = Scan::default();
        let mut counts = Measurement::default();
        scan_records(
            &records,
            &config(),
            &policy(),
            false,
            &mut scan,
            &mut counts,
        )
        .unwrap();
        assert_eq!(
            scan.hits
                .iter()
                .map(|hit| hit.url.as_str())
                .collect::<Vec<_>>(),
            vec!["edited-old", "new"]
        );
        assert_eq!(scan.hits[0].line, 2);
        assert!(!format!("{:?}", scan.hits).contains("restricted-example"));
    }
    #[test]
    fn public_names_and_process_words_are_not_private_names() {
        assert!(
            scan_private_line(
                "Onsager Semon Ostrom Ostrom Hub dogfood CODEOWNERS pre-push",
                PRIVATE_NAMES
            )
            .is_empty()
        );
        assert!(skill_rule_ids("https://github.com/onsager-ai/duhem on GitHub").is_empty());
    }
    #[test]
    fn measurement_ignores_cutoff_and_counts_both_policies_without_locations() {
        let records = json!([{"user":{"login":"configured"},"html_url":"do-not-output", "updated_at":"2025-01-01T00:00:00Z", "body":"dogfood restricted-example"}]);
        let mut scan = Scan::default();
        let mut counts = Measurement::default();
        scan_records(&records, &config(), &policy(), true, &mut scan, &mut counts).unwrap();
        assert!(scan.hits.is_empty());
        assert_eq!(counts.skill_vocabulary["S01"][&2025], 1);
        assert_eq!(counts.private_names["test"][&2025], 1);
    }
    #[test]
    fn exact_merge_day_timestamp_excludes_earlier_activity_on_the_same_day() {
        let mut config = config();
        config.since = DateTime::parse_from_rfc3339("2026-10-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let records = json!([
            {"user":{"login":"configured"},"html_url":"earlier-today","updated_at":"2026-10-30T11:59:59Z","body":"restricted-example"},
            {"user":{"login":"configured"},"html_url":"at-baseline","updated_at":"2026-10-30T12:00:00Z","body":"restricted-example"}
        ]);
        let mut scan = Scan::default();
        let mut counts = Measurement::default();
        scan_records(&records, &config, &policy(), false, &mut scan, &mut counts).unwrap();
        assert_eq!(
            scan.hits
                .iter()
                .map(|hit| hit.url.as_str())
                .collect::<Vec<_>>(),
            vec!["at-baseline"]
        );
    }

    #[test]
    fn malformed_payload_or_missing_update_time_is_not_a_clean_scan() {
        let mut scan = Scan::default();
        let mut counts = Measurement::default();
        assert!(
            scan_records(
                &Value::Null,
                &config(),
                &policy(),
                false,
                &mut scan,
                &mut counts
            )
            .is_err()
        );
        let record = json!([{"user":{"login":"configured"},"body":"restricted-example"}]);
        assert!(
            scan_records(&record, &config(), &policy(), false, &mut scan, &mut counts).is_err()
        );
    }
}
