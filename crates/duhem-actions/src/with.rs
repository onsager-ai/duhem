//! Shared `with:` helpers — currently just `WithinSpec`, the duration
//! shape every UI action accepts under `within:`.
//!
//! Accepts plain integer milliseconds (`200`) or a string with a
//! `ms` / `s` / `m` suffix (`200ms`, `2s`, `1m`). Returned as
//! `std::time::Duration`. Floats are deliberately rejected — the
//! source of truth is YAML, not human-typed Rust literals.

use std::time::Duration;

use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WithinSpec(pub Duration);

impl From<WithinSpec> for Duration {
    fn from(w: WithinSpec) -> Self {
        w.0
    }
}

impl<'de> Deserialize<'de> for WithinSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{Error, Unexpected};

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(u64),
            Str(String),
        }

        match Raw::deserialize(d)? {
            Raw::Int(ms) => Ok(WithinSpec(Duration::from_millis(ms))),
            Raw::Str(s) => parse_duration_str(&s).map(WithinSpec).ok_or_else(|| {
                D::Error::invalid_value(Unexpected::Str(&s), &"like `200ms`, `2s`, `1m`")
            }),
        }
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
    fn parses_int_ms() {
        let w: WithinSpec = serde_yml::from_str("250").unwrap();
        assert_eq!(Duration::from(w), Duration::from_millis(250));
    }

    #[test]
    fn parses_ms_suffix() {
        let w: WithinSpec = serde_yml::from_str("\"200ms\"").unwrap();
        assert_eq!(Duration::from(w), Duration::from_millis(200));
    }

    #[test]
    fn parses_seconds() {
        let w: WithinSpec = serde_yml::from_str("2s").unwrap();
        assert_eq!(Duration::from(w), Duration::from_secs(2));
    }

    #[test]
    fn parses_minutes() {
        let w: WithinSpec = serde_yml::from_str("1m").unwrap();
        assert_eq!(Duration::from(w), Duration::from_secs(60));
    }

    #[test]
    fn rejects_garbage() {
        assert!(serde_yml::from_str::<WithinSpec>("\"forever\"").is_err());
    }

    #[test]
    fn rejects_minutes_overflow() {
        // u64::MAX minutes would overflow when multiplied by 60.
        let huge = format!("\"{}m\"", u64::MAX);
        assert!(serde_yml::from_str::<WithinSpec>(&huge).is_err());
    }
}
