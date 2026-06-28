//! Named-environment selection + projection for `duhem run` (spec #68).
//!
//! A root manifest may declare an `environments:` block — named
//! environment configs (free-form `key: value` maps). For a given run
//! exactly one environment is *selected*; its keys then participate on
//! two surfaces:
//!
//! - **Input resolution.** A selected key `base_url` populates a
//!   declared input `base_url` when no higher-precedence source
//!   (`--inputs`, `--inputs-file`) supplies it. The values flow as
//!   `serde_json::Value` so they share the existing typed-input
//!   resolution path (`main::resolve_inputs`, precedence layer 3).
//! - **`$env.<key>` whitelist.** The selected environment's
//!   *string-valued* keys seed the runtime's `$env` whitelist for that
//!   run (`Engine::with_env`). Non-string values are usable as input
//!   defaults but do not enter the `$env` string map — `$env.<key>` is
//!   a string surface.
//!
//! Selection rule (no `defaults.environment` yet — that's sibling #66):
//!
//! - `--environment <name>` given → use it; error if it names an
//!   environment the manifest does not declare (listing the available
//!   ones).
//! - else exactly one environment declared → auto-select it.
//! - else zero environments → nothing selected (full back-compat).
//! - else (two or more, no `--environment`) → error requiring an
//!   explicit `--environment`.
//!
//! A single-leaf run has no manifest, so no environment applies; a
//! `--environment` passed there is inert (the caller warns).

use std::collections::BTreeMap;

use crate::inputs::yml_to_json;

/// The selected environment, projected onto the two surfaces the run
/// consumes: a JSON map for input resolution, and a string map for the
/// `$env` whitelist.
#[derive(Debug)]
pub struct SelectedEnvironment {
    /// The selected environment's name (for diagnostics).
    pub name: String,
    /// Every key, converted to `serde_json::Value`, for the input
    /// resolution chain (precedence layer 3).
    pub inputs: BTreeMap<String, serde_json::Value>,
    /// Only the *string-valued* keys, for the `$env.<key>` whitelist.
    pub env: BTreeMap<String, String>,
}

/// Pick the environment for this run from the manifest's declared
/// `environments:` block and the optional `--environment` flag.
///
/// Returns `Ok(None)` when nothing is selected (zero environments
/// declared and no flag) — that path is byte-identical to today.
/// Returns `Err(message)` for an unknown requested name or for an
/// ambiguous (2+) declaration with no flag; the message mirrors the
/// CLI's existing error style.
pub fn select_environment(
    environments: &BTreeMap<String, BTreeMap<String, serde_yml::Value>>,
    requested: Option<&str>,
) -> Result<Option<SelectedEnvironment>, String> {
    let chosen: Option<&str> = match requested {
        Some(name) => {
            if !environments.contains_key(name) {
                return Err(format!(
                    "--environment `{name}`: no such environment{}",
                    available_suffix(environments)
                ));
            }
            Some(name)
        }
        None => match environments.len() {
            0 => None,
            1 => environments.keys().next().map(String::as_str),
            _ => {
                let names = environments.keys().cloned().collect::<Vec<_>>().join(", ");
                return Err(format!(
                    "multiple environments declared ({names}); select one with --environment"
                ));
            }
        },
    };

    let Some(name) = chosen else {
        return Ok(None);
    };
    let keys = &environments[name];
    project(name, keys).map(Some)
}

/// Project a selected environment's raw key map onto the input-JSON +
/// `$env`-string surfaces.
fn project(
    name: &str,
    keys: &BTreeMap<String, serde_yml::Value>,
) -> Result<SelectedEnvironment, String> {
    let mut inputs = BTreeMap::new();
    let mut env = BTreeMap::new();
    for (k, v) in keys {
        let json = yml_to_json(v).map_err(|e| format!("environment `{name}`: key `{k}`: {e}"))?;
        // Only string-valued keys enter the `$env` string whitelist;
        // `$env.<key>` is a string surface. Non-string values still
        // flow into `inputs` so they can feed typed input defaults.
        if let serde_json::Value::String(s) = &json {
            env.insert(k.clone(), s.clone());
        }
        inputs.insert(k.clone(), json);
    }
    Ok(SelectedEnvironment {
        name: name.to_string(),
        inputs,
        env,
    })
}

fn available_suffix(environments: &BTreeMap<String, BTreeMap<String, serde_yml::Value>>) -> String {
    if environments.is_empty() {
        " (the manifest declares no environments)".to_string()
    } else {
        let names = environments.keys().cloned().collect::<Vec<_>>().join(", ");
        format!(" (available: {names})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envs(
        pairs: &[(&str, &[(&str, serde_yml::Value)])],
    ) -> BTreeMap<String, BTreeMap<String, serde_yml::Value>> {
        pairs
            .iter()
            .map(|(name, keys)| {
                let m = keys
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect();
                (name.to_string(), m)
            })
            .collect()
    }

    fn s(v: &str) -> serde_yml::Value {
        serde_yml::Value::String(v.to_string())
    }

    #[test]
    fn explicit_flag_selects_named_environment() {
        let e = envs(&[
            ("staging", &[("base_url", s("https://staging"))]),
            ("prod", &[("base_url", s("https://prod"))]),
        ]);
        let sel = select_environment(&e, Some("prod")).unwrap().unwrap();
        assert_eq!(sel.name, "prod");
        assert_eq!(sel.inputs["base_url"], serde_json::json!("https://prod"));
        assert_eq!(sel.env["base_url"], "https://prod");
    }

    #[test]
    fn unknown_flag_errors_listing_available() {
        let e = envs(&[
            ("staging", &[("base_url", s("https://staging"))]),
            ("prod", &[("base_url", s("https://prod"))]),
        ]);
        let err = select_environment(&e, Some("dev")).unwrap_err();
        assert!(err.contains("no such environment"), "{err}");
        assert!(err.contains("staging"), "{err}");
        assert!(err.contains("prod"), "{err}");
    }

    #[test]
    fn single_environment_auto_selects() {
        let e = envs(&[("staging", &[("base_url", s("https://staging"))])]);
        let sel = select_environment(&e, None).unwrap().unwrap();
        assert_eq!(sel.name, "staging");
    }

    #[test]
    fn multiple_environments_without_flag_errors() {
        let e = envs(&[
            ("staging", &[("base_url", s("https://staging"))]),
            ("prod", &[("base_url", s("https://prod"))]),
        ]);
        let err = select_environment(&e, None).unwrap_err();
        assert!(err.contains("multiple environments"), "{err}");
        assert!(err.contains("--environment"), "{err}");
    }

    #[test]
    fn zero_environments_select_nothing() {
        let e = envs(&[]);
        assert!(select_environment(&e, None).unwrap().is_none());
    }

    #[test]
    fn non_string_values_skip_env_whitelist_but_feed_inputs() {
        let e = envs(&[(
            "staging",
            &[
                ("base_url", s("https://staging")),
                ("workers", serde_yml::Value::Number(3.into())),
            ],
        )]);
        let sel = select_environment(&e, Some("staging")).unwrap().unwrap();
        // string key is on both surfaces
        assert_eq!(sel.env["base_url"], "https://staging");
        // non-string key feeds inputs only
        assert_eq!(sel.inputs["workers"], serde_json::json!(3));
        assert!(!sel.env.contains_key("workers"));
    }
}
