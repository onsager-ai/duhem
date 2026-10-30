use std::collections::BTreeMap;
use std::time::Duration;

use duhem_actions::{ActionError, TimeoutSpec};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Launch {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub ready: Option<Ready>,
    pub stop_timeout: Option<TimeoutSpec>,
    pub kill_timeout: Option<TimeoutSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ready {
    pub http: Option<Http>,
    pub log: Option<Log>,
    pub timeout: TimeoutSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Http {
    pub url: String,
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Log {
    pub stream: String,
    pub pattern: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Handle {
    pub handle: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inspect {
    pub handle: String,
    #[serde(default)]
    pub cursor: u64,
}

impl Launch {
    pub fn validate(&self) -> Result<(), ActionError> {
        if self.command.is_empty() {
            return Err(error("command must not be empty"));
        }
        for (name, timeout) in [
            ("stop_timeout", self.stop_timeout),
            ("kill_timeout", self.kill_timeout),
        ] {
            if timeout
                .is_some_and(|timeout| timeout.0.is_zero() || timeout.0 > Duration::from_secs(300))
            {
                return Err(error(format!(
                    "{name} must be greater than zero and at most 5m"
                )));
            }
        }
        if let Some(ready) = &self.ready {
            if ready.http.is_some() == ready.log.is_some() {
                return Err(error("ready requires exactly one of http or log"));
            }
            if ready.timeout.0.is_zero() || ready.timeout.0 > Duration::from_secs(300) {
                return Err(error(
                    "ready.timeout must be greater than zero and at most 5m",
                ));
            }
            if let Some(log) = &ready.log {
                if !matches!(log.stream.as_str(), "stdout" | "stderr") {
                    return Err(error("ready.log.stream must be stdout or stderr"));
                }
                regex::Regex::new(&log.pattern)
                    .map_err(|err| error(format!("invalid readiness regex: {err}")))?;
            }
            if let Some(http) = &ready.http {
                let url = reqwest::Url::parse(&http.url)
                    .map_err(|err| error(format!("invalid readiness URL: {err}")))?;
                if !matches!(url.scheme(), "http" | "https") {
                    return Err(error("readiness URL must use http or https"));
                }
                reqwest::StatusCode::from_u16(http.status).map_err(|err| error(err.to_string()))?;
                for (key, value) in &http.headers {
                    reqwest::header::HeaderName::from_bytes(key.as_bytes())
                        .map_err(|err| error(err.to_string()))?;
                    reqwest::header::HeaderValue::from_str(value)
                        .map_err(|err| error(err.to_string()))?;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn error(message: impl Into<String>) -> ActionError {
    ActionError::Process(message.into())
}

pub(super) fn parse<T: serde::de::DeserializeOwned>(
    uses: &'static str,
    with: &serde_yml::Value,
) -> Result<T, ActionError> {
    serde_yml::from_value(with.clone()).map_err(|source| ActionError::InvalidWith {
        action: uses,
        source,
    })
}
