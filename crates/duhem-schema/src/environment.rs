//! Operator-supplied environment provisioning hooks.
//!
//! Stage 3 of `docs/duhem-spec.md` §9 — "Provision Environment" — was
//! implicit before this landed: `duhem run` assumed the SUT was
//! already up. v1 closes that gap with operator-supplied scripts and
//! a readiness probe the runtime sequences around `setup:` and the
//! criteria loop (spec on issue #50). The schema only declares the
//! wire shape; the runtime spec owns lifecycle.

use std::path::PathBuf;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Operator-supplied lifecycle hooks for the system-under-test.
///
/// `up:` runs once before `setup:`; `down:` (if declared) runs once
/// after the last criterion, regardless of verdict. `ready:` is a
/// readiness probe the runtime polls between `up:` exiting zero and
/// `setup:` starting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// Path to an executable that brings the SUT up. Relative paths
    /// are resolved against the directory containing the Verification
    /// Definition.
    pub up: PathBuf,

    /// Optional path to an executable that tears the SUT down. Runs
    /// after the criteria loop regardless of verdict. Teardown
    /// failures are evidence, not verdict-altering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<PathBuf>,

    /// Optional readiness probe. Polled after `up:` exits zero and
    /// before `setup:` starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<ReadyProbe>,
}

/// The closed set of readiness probes. v1 ships `http:` only; TCP,
/// gRPC health, etc. are follow-up additions per the issue Out-of-scope.
///
/// Wire shape:
///
/// ```yaml
/// ready:
///   http:
///     url: http://localhost:3000/healthz
///     timeout: 60s
/// ```
///
/// Modeled as a struct with optional kind-keyed fields rather than a
/// serde externally-tagged enum because `serde_yml`'s default
/// externally-tagged form is the `!tag content` syntax, not the
/// `{kind: content}` map shape the spec on issue #50 calls out.
/// Exactly-one-set is enforced at deserialize time so the closed-enum
/// guarantee holds at the schema layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyProbe {
    Http(HttpReadyProbe),
}

impl<'de> Deserialize<'de> for ReadyProbe {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            #[serde(default)]
            http: Option<HttpReadyProbe>,
        }

        let raw = Raw::deserialize(d)?;
        match raw.http {
            Some(p) => Ok(ReadyProbe::Http(p)),
            None => Err(D::Error::custom("ready: expected exactly one of: `http`")),
        }
    }
}

impl Serialize for ReadyProbe {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            ReadyProbe::Http(p) => {
                let mut m = ser.serialize_map(Some(1))?;
                m.serialize_entry("http", p)?;
                m.end()
            }
        }
    }
}

/// HTTP readiness probe. `url:` is templated at runtime (the runtime
/// resolves `$inputs.<name>` references); the schema layer treats it
/// as an opaque string so probes that don't need substitution stay
/// portable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpReadyProbe {
    /// URL to GET. May contain a single whole-string `$inputs.<name>`
    /// reference, resolved by the runtime before the first poll.
    pub url: String,

    /// Status code that signals readiness. Defaults to 200.
    #[serde(default = "default_expect_status")]
    pub expect_status: u16,

    /// Total time to keep polling before giving up. The runtime maps
    /// timeout to `Outcome::Timeout` → run verdict `Inconclusive`.
    pub timeout: DurationSpec,
}

fn default_expect_status() -> u16 {
    200
}

/// Duration wire format shared with `duhem-actions::WithinSpec`:
/// integer milliseconds (`200`) or a string with a `ms` / `s` / `m`
/// suffix (`60s`, `2m`). Defined locally so the schema crate does
/// not pull in `duhem-actions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurationSpec(pub Duration);

impl From<DurationSpec> for Duration {
    fn from(d: DurationSpec) -> Self {
        d.0
    }
}

impl<'de> Deserialize<'de> for DurationSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{Error, Unexpected};

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(u64),
            Str(String),
        }

        match Raw::deserialize(d)? {
            Raw::Int(ms) => Ok(DurationSpec(Duration::from_millis(ms))),
            Raw::Str(s) => parse_duration_str(&s).map(DurationSpec).ok_or_else(|| {
                D::Error::invalid_value(Unexpected::Str(&s), &"like `200ms`, `60s`, `2m`")
            }),
        }
    }
}

impl Serialize for DurationSpec {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        // Re-emit as the millisecond integer form. The string suffix
        // form survives round-trip semantically (same Duration) even
        // though the raw text changes.
        let ms = self.0.as_millis();
        if ms > u64::MAX as u128 {
            return Err(serde::ser::Error::custom(
                "duration exceeds u64 milliseconds",
            ));
        }
        ser.serialize_u64(ms as u64)
    }
}

fn parse_duration_str(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (num_part, suffix): (&str, &str) = if let Some(rest) = s.strip_suffix("ms") {
        (rest, "ms")
    } else if let Some(rest) = s.strip_suffix('s') {
        (rest, "s")
    } else if let Some(rest) = s.strip_suffix('m') {
        (rest, "m")
    } else {
        return None;
    };
    let n: u64 = num_part.trim().parse().ok()?;
    Some(match suffix {
        "ms" => Duration::from_millis(n),
        "s" => Duration::from_secs(n),
        "m" => Duration::from_secs(n.checked_mul(60)?),
        _ => unreachable!(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_environment() {
        let y = "up: ./scripts/up.sh\n";
        let env: Environment = serde_yml::from_str(y).unwrap();
        assert_eq!(env.up, PathBuf::from("./scripts/up.sh"));
        assert!(env.down.is_none());
        assert!(env.ready.is_none());
    }

    #[test]
    fn parses_full_environment() {
        let y = r#"
up: ./scripts/up.sh
down: ./scripts/down.sh
ready:
  http:
    url: http://localhost:3000/healthz
    expect_status: 200
    timeout: 60s
"#;
        let env: Environment = serde_yml::from_str(y).unwrap();
        assert_eq!(env.up, PathBuf::from("./scripts/up.sh"));
        assert_eq!(env.down, Some(PathBuf::from("./scripts/down.sh")));
        let ReadyProbe::Http(probe) = env.ready.unwrap();
        assert_eq!(probe.url, "http://localhost:3000/healthz");
        assert_eq!(probe.expect_status, 200);
        assert_eq!(Duration::from(probe.timeout), Duration::from_secs(60));
    }

    #[test]
    fn ready_http_defaults_expect_status_to_200() {
        let y = r#"
up: ./up.sh
ready:
  http:
    url: http://x/health
    timeout: 5s
"#;
        let env: Environment = serde_yml::from_str(y).unwrap();
        let ReadyProbe::Http(probe) = env.ready.unwrap();
        assert_eq!(probe.expect_status, 200);
    }

    #[test]
    fn missing_up_is_a_parse_error() {
        // `up:` is required when `environment:` is present — modeled
        // as a non-optional field, so serde produces the error.
        let y = "down: ./down.sh\n";
        assert!(serde_yml::from_str::<Environment>(y).is_err());
    }

    #[test]
    fn unknown_field_under_environment_is_rejected() {
        let y = "up: ./up.sh\nbogus: 1\n";
        let err = serde_yml::from_str::<Environment>(y).unwrap_err();
        assert!(format!("{err}").contains("unknown field"), "got: {err}");
    }

    #[test]
    fn unknown_ready_kind_is_rejected() {
        let y = r#"
up: ./up.sh
ready:
  tcp:
    host: localhost
    port: 5432
"#;
        assert!(serde_yml::from_str::<Environment>(y).is_err());
    }

    #[test]
    fn duration_accepts_int_ms_and_suffixed_strings() {
        for (src, expected_ms) in [
            ("250", 250),
            ("\"200ms\"", 200),
            ("\"3s\"", 3_000),
            ("\"2m\"", 120_000),
        ] {
            let d: DurationSpec = serde_yml::from_str(src).unwrap();
            assert_eq!(
                Duration::from(d),
                Duration::from_millis(expected_ms),
                "for `{src}`"
            );
        }
    }
}
