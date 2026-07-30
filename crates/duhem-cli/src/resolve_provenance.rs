//! Source tracking for `duhem resolve`.
//!
//! The production loader intentionally returns only the effective
//! manifest. This read-only companion walks the same root-wins include
//! order and records where input declarations and profile values came
//! from, including declarations that lost to an earlier source.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Origin {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ValueProvenance {
    pub rung: String,
    #[serde(flatten)]
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overridden: Vec<Origin>,
}

#[derive(Debug, Default)]
pub(crate) struct ManifestOrigins {
    pub inputs: BTreeMap<String, ValueProvenance>,
    pub profiles: BTreeMap<(String, String), ValueProvenance>,
}

pub(crate) fn collect_manifest_origins(path: &Path) -> ManifestOrigins {
    let mut out = ManifestOrigins::default();
    collect_file(path, true, &mut out, &mut Vec::new());
    out
}

fn collect_file(path: &Path, root: bool, out: &mut ManifestOrigins, chain: &mut Vec<PathBuf>) {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if chain.contains(&canonical) {
        return;
    }
    let Ok(src) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(value) = serde_yml::from_str::<serde_yml::Value>(&src) else {
        return;
    };

    chain.push(canonical);
    if !root {
        collect_includes(path, &value, out, chain);
    }
    record_sections(path, &value, out);
    if root {
        collect_includes(path, &value, out, chain);
    }
    chain.pop();
}

fn collect_includes(
    path: &Path,
    value: &serde_yml::Value,
    out: &mut ManifestOrigins,
    chain: &mut Vec<PathBuf>,
) {
    let Some(includes) = mapping_value(value, "includes").and_then(serde_yml::Value::as_sequence)
    else {
        return;
    };
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    for include in includes {
        if let Some(relative) = include.as_str() {
            collect_file(&parent.join(relative), false, out, chain);
        }
    }
}

fn record_sections(path: &Path, value: &serde_yml::Value, out: &mut ManifestOrigins) {
    if let Some(inputs) = mapping_value(value, "inputs").and_then(serde_yml::Value::as_mapping) {
        for (name, _) in inputs {
            let Some(name) = name.as_str() else {
                continue;
            };
            let origin = origin(path, &["inputs", name]);
            record(
                &mut out.inputs,
                name.to_string(),
                "manifest declaration",
                origin,
            );
        }
    }

    let Some(profiles) = mapping_value(value, "profiles").and_then(serde_yml::Value::as_mapping)
    else {
        return;
    };
    for (profile, keys) in profiles {
        let (Some(profile), Some(keys)) = (profile.as_str(), keys.as_mapping()) else {
            continue;
        };
        for (key, _) in keys {
            let Some(key) = key.as_str() else {
                continue;
            };
            let source = origin(path, &["profiles", profile, key]);
            record(
                &mut out.profiles,
                (profile.to_string(), key.to_string()),
                format!("profile {profile}"),
                source,
            );
        }
    }
}

fn record<K: Ord>(
    map: &mut BTreeMap<K, ValueProvenance>,
    key: K,
    rung: impl Into<String>,
    source: Origin,
) {
    if let Some(winner) = map.get_mut(&key) {
        winner.overridden.push(source);
    } else {
        map.insert(
            key,
            ValueProvenance {
                rung: rung.into(),
                origin: source,
                overridden: Vec::new(),
            },
        );
    }
}

pub(crate) fn input_override_origins(tokens: &[String]) -> BTreeMap<String, ValueProvenance> {
    let mut out = BTreeMap::new();
    for token in tokens {
        if let Some(raw_path) = token.strip_prefix('@')
            && !token.contains('=')
        {
            let path = Path::new(raw_path);
            if let Ok(values) = crate::inputs::load_inputs_file(path) {
                for name in values.keys() {
                    override_origin(
                        &mut out,
                        name.clone(),
                        ValueProvenance {
                            rung: "--inputs @file".to_string(),
                            origin: origin(path, &[name]),
                            overridden: Vec::new(),
                        },
                    );
                }
            }
        } else if let Some((name, _)) = token.split_once('=') {
            override_origin(
                &mut out,
                name.to_string(),
                ValueProvenance {
                    rung: "--inputs".to_string(),
                    origin: Origin {
                        source: "command line".to_string(),
                        line: None,
                    },
                    overridden: Vec::new(),
                },
            );
        }
    }
    out
}

fn override_origin(
    map: &mut BTreeMap<String, ValueProvenance>,
    name: String,
    mut winner: ValueProvenance,
) {
    if let Some(previous) = map.remove(&name) {
        winner.overridden.push(previous.origin);
        winner.overridden.extend(previous.overridden);
    }
    map.insert(name, winner);
}

pub(crate) fn origin(path: &Path, yaml_path: &[&str]) -> Origin {
    Origin {
        source: path.display().to_string(),
        line: find_key_line(path, yaml_path),
    }
}

fn mapping_value<'a>(value: &'a serde_yml::Value, key: &str) -> Option<&'a serde_yml::Value> {
    value
        .as_mapping()?
        .get(serde_yml::Value::String(key.to_string()))
}

/// Locate a mapping key using YAML indentation. Provenance is advisory
/// presentation metadata; malformed YAML simply yields no line number.
pub(crate) fn find_key_line(path: &Path, wanted: &[&str]) -> Option<usize> {
    let src = std::fs::read_to_string(path).ok()?;
    let mut stack: Vec<(usize, String)> = Vec::new();
    for (index, raw) in src.lines().enumerate() {
        let content = raw.split('#').next()?.trim_end();
        if content.trim().is_empty() || content.trim_start().starts_with('-') {
            continue;
        }
        let indent = content.len().saturating_sub(content.trim_start().len());
        let trimmed = content.trim_start();
        let Some((key, _)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches(['\'', '"']);
        if key.is_empty() {
            continue;
        }
        while stack.last().is_some_and(|(prior, _)| *prior >= indent) {
            stack.pop();
        }
        stack.push((indent, key.to_string()));
        if stack.len() == wanted.len()
            && stack
                .iter()
                .map(|(_, key)| key.as_str())
                .eq(wanted.iter().copied())
        {
            return Some(index + 1);
        }
    }
    None
}
