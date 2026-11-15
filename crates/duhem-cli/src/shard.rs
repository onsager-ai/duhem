//! CI-neutral shard detection and deterministic, criterion-sized partitioning.

use std::collections::BTreeSet;

use duhem_summary::ShardIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub shard: ShardIdentity,
    pub identity: String,
}

/// Resolve each explicit axis independently: flags, generic environment, CI.
/// The environment reader is supplied so unit tests never mutate process state.
pub fn detect(
    flag: Option<&str>,
    run_id: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<Detection>, String> {
    let generic = env("DUHEM_SHARD");
    let (index, total, source, ci_identity) = if let Some(value) = flag.or(generic.as_deref()) {
        let (index, total) = parse(value)?;
        let source = if flag.is_some() { "flag" } else { "env" };
        (index, total, source, ci_identity(&env))
    } else if let Some(index) = env("CI_NODE_INDEX") {
        let total = required(&env, "CI_NODE_TOTAL")?;
        let (index, total) = parse(&format!("{index}/{total}"))?;
        (index, total, "gitlab", ci_identity(&env))
    } else if let Some(index) = env("BUILDKITE_PARALLEL_JOB") {
        let total = required(&env, "BUILDKITE_PARALLEL_JOB_COUNT")?;
        let (index, total) = zero_based(&index, &total)?;
        (index, total, "buildkite", ci_identity(&env))
    } else if let Some(index) = env("CIRCLE_NODE_INDEX") {
        let total = required(&env, "CIRCLE_NODE_TOTAL")?;
        let (index, total) = zero_based(&index, &total)?;
        (index, total, "circleci", ci_identity(&env))
    } else {
        // Totals alone are common on nonparallel CI jobs (notably GitLab).
        return Ok(None);
    };
    let identity = run_id.map(str::to_owned).or_else(|| env("DUHEM_RUN_ID"))
        .or(ci_identity)
        .ok_or("sharding requires a shared --run-id or DUHEM_RUN_ID outside a supported CI job")?;
    if identity.trim().is_empty() {
        return Err("shard run identity must not be empty".into());
    }
    Ok(Some(Detection {
        shard: ShardIdentity { index, total, source: source.into() },
        identity,
    }))
}

fn required(env: &impl Fn(&str) -> Option<String>, key: &str) -> Result<String, String> {
    env(key).filter(|value| !value.is_empty()).ok_or_else(|| format!("shard detection requires {key}"))
}

fn ci_identity(env: &impl Fn(&str) -> Option<String>) -> Option<String> {
    let keys: &[&str] = if env("CI_NODE_INDEX").is_some() || env("CI_PIPELINE_ID").is_some() {
        &["CI_PIPELINE_ID", "CI_JOB_NAME"]
    } else if env("BUILDKITE_BUILD_ID").is_some() {
        &["BUILDKITE_BUILD_ID", "BUILDKITE_STEP_ID"]
    } else if env("CIRCLE_WORKFLOW_ID").is_some() {
        &["CIRCLE_WORKFLOW_ID", "CIRCLE_JOB"]
    } else {
        &["GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "GITHUB_JOB"]
    };
    let mut values: Vec<String> = keys.iter().map(|key| required(env, key).ok()).collect::<Option<_>>()?;
    if keys[0] == "CI_PIPELINE_ID"
        && let Some((name, suffix)) = values[1].rsplit_once(' ')
        && parse(suffix).is_ok()
    {
        values[1] = name.to_owned();
    }
    // JSON framing prevents delimiter collisions between identity components.
    Some(serde_json::to_string(&(keys[0], values)).expect("string identity serializes"))
}

fn zero_based(index: &str, total: &str) -> Result<(usize, usize), String> {
    let index = index.parse::<usize>().ok().and_then(|index| index.checked_add(1))
        .ok_or_else(|| format!("invalid zero-based shard index `{index}`"))?;
    parse(&format!("{index}/{total}"))
}

pub fn parse(value: &str) -> Result<(usize, usize), String> {
    let invalid = || format!("invalid shard `{value}`: expected k/n with 1 <= k <= n");
    let (index, total) = value.split_once('/').ok_or_else(invalid)?;
    if !index.bytes().all(|byte| byte.is_ascii_digit()) || !total.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let index = index.parse::<usize>().map_err(|_| invalid())?;
    let total = total.parse::<usize>().map_err(|_| invalid())?;
    if index == 0 || total == 0 || index > total { return Err(invalid()); }
    Ok((index, total))
}

/// Sort stable unit IDs and assign round-robin. This balances unit counts to
/// within one without depending on process hashes, manifest glob order or paths
/// outside the manifest root. IDs must be unique within the selected run.
pub fn partition(units: &[String], index: usize, total: usize) -> BTreeSet<String> {
    assert!(index > 0 && index <= total);
    let ordered: BTreeSet<_> = units.iter().cloned().collect();
    ordered.into_iter().enumerate().filter_map(|(offset, unit)|
        (offset % total == index - 1).then_some(unit)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detected(vars: &[(&str, &str)], flag: Option<&str>, id: Option<&str>) -> Result<Option<Detection>, String> {
        detect(flag, id, |key| vars.iter().find(|(name, _)| *name == key).map(|(_, value)| (*value).into()))
    }

    #[test]
    fn ci_table_and_precedence() {
        let cases: &[(&[(&str, &str)], &str)] = &[
            (&[("CI_NODE_INDEX", "1"), ("CI_NODE_TOTAL", "3"), ("CI_PIPELINE_ID", "42"), ("CI_JOB_NAME", "test 1/3")], "gitlab"),
            (&[("BUILDKITE_PARALLEL_JOB", "0"), ("BUILDKITE_PARALLEL_JOB_COUNT", "3"), ("BUILDKITE_BUILD_ID", "42"), ("BUILDKITE_STEP_ID", "test")], "buildkite"),
            (&[("CIRCLE_NODE_INDEX", "0"), ("CIRCLE_NODE_TOTAL", "3"), ("CIRCLE_WORKFLOW_ID", "42"), ("CIRCLE_JOB", "test")], "circleci"),
            (&[("DUHEM_SHARD", "1/3"), ("GITHUB_RUN_ID", "42"), ("GITHUB_RUN_ATTEMPT", "1"), ("GITHUB_JOB", "test")], "env"),
        ];
        for (vars, source) in cases {
            let found = detected(vars, None, None).unwrap().unwrap();
            assert_eq!((found.shard.index, found.shard.total), (1, 3));
            assert_eq!(found.shard.source, *source);
            let override_value = detected(vars, Some("2/4"), Some("override")).unwrap().unwrap();
            assert_eq!(override_value.shard.index, 2);
            assert_eq!(override_value.shard.source, "flag");
            assert_eq!(override_value.identity, "override");
        }
        let a = detected(cases[0].0, None, None).unwrap().unwrap();
        let b = detected(&[("CI_NODE_INDEX", "2"), ("CI_NODE_TOTAL", "3"), ("CI_PIPELINE_ID", "42"), ("CI_JOB_NAME", "test 2/3")], None, None).unwrap().unwrap();
        assert_eq!(a.identity, b.identity);
        let generic = detected(&[("DUHEM_SHARD", "2/3"), ("DUHEM_RUN_ID", "generic"), ("CI_NODE_INDEX", "broken")], None, None).unwrap().unwrap();
        assert_eq!(generic.identity, "generic");
        assert_eq!(generic.shard.source, "env");
    }

    #[test]
    fn rejects_invalid_values_and_requires_identity() {
        for value in ["", "0/2", "3/2", "1/0", "-1/3", "1", "1/2/3", "1 /2", "+1/2"] {
            assert!(parse(value).is_err(), "{value}");
        }
        assert!(detected(&[], Some("1/2"), None).is_err());
        assert!(detected(&[("CI_NODE_INDEX", "1")], None, Some("id")).is_err());
        assert!(detected(&[], None, Some("id")).unwrap().is_none());
        assert!(detected(&[("CI_NODE_TOTAL", "1")], None, None).unwrap().is_none());
    }

    #[test]
    fn partition_is_deterministic_disjoint_covering_and_balanced() {
        for count in [0, 1, 2, 7, 17, 39] {
            let units: Vec<_> = (0..count).map(|i| format!("leaf-{}/criterion-{i}", i % 3)).collect();
            for n in 1..=9 {
                let mut union = BTreeSet::new();
                let mut sizes = Vec::new();
                for k in 1..=n {
                    let selected = partition(&units, k, n);
                    let reversed: Vec<_> = units.iter().rev().cloned().collect();
                    assert_eq!(selected, partition(&reversed, k, n));
                    assert!(union.is_disjoint(&selected));
                    sizes.push(selected.len());
                    union.extend(selected);
                }
                assert_eq!(union, units.iter().cloned().collect());
                assert!(sizes.iter().max().unwrap() - sizes.iter().min().unwrap() <= 1);
            }
        }
    }
}
