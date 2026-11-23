//! A single provider call that records scoring evidence.
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::action::DEFAULT_TIMEOUT;
use crate::{Action, ActionContract, ActionCtx, ActionError, ActionResult, FieldSpec, TimeoutSpec};

pub struct Score;

const USES: &str = duhem_schema::score::ACTION;
const MAX_TOKENS: u32 = 1024;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Provider {
    Anthropic,
    Openai,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct With {
    provider: Provider,
    model: String,
    base_url: Option<String>,
    api_key: String,
    rubric: String,
    observations: Vec<Value>,
    range: Range,
    timeout: Option<TimeoutSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Range {
    min: f64,
    max: f64,
}

#[derive(Deserialize)]
struct Reply {
    score: f64,
    reason: String,
}

impl Provider {
    fn route(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Anthropic => ("anthropic", "https://api.anthropic.com/v1", "messages"),
            Self::Openai => ("openai", "https://api.openai.com/v1", "chat/completions"),
        }
    }

    fn content(self, response: &Value) -> Option<&str> {
        match self {
            Self::Anthropic => response
                .get("content")?
                .as_array()?
                .iter()
                .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))?
                .get("text")?
                .as_str(),
            Self::Openai => response.pointer("/choices/0/message/content")?.as_str(),
        }
    }
}

fn transport_error(result: ActionResult, error: reqwest::Error) -> ActionResult {
    if error.is_timeout() {
        return ActionResult {
            outcome: crate::Outcome::Timeout,
            ..result
        }
        .with_output("missing_cause", json!("provider request timed out"));
    }
    result.with_output(
        "missing_cause",
        json!(format!("provider request failed: {}", error.without_url())),
    )
}

#[async_trait]
impl Action for Score {
    fn uses(&self) -> &'static str {
        USES
    }

    fn contract(&self) -> ActionContract {
        ActionContract {
            uses: USES,
            summary: "Record a rubric score as evidence without contributing to the verdict.",
            with: vec![
                FieldSpec::enum_of("provider", true, &["anthropic", "openai"]),
                FieldSpec::required("model"),
                FieldSpec::optional("base_url"),
                FieldSpec::required("api_key"),
                FieldSpec::required("rubric"),
                FieldSpec::required("observations"),
                FieldSpec::required("range"),
                FieldSpec::optional("timeout"),
            ],
            outputs: vec![],
            secret_outputs: vec![],
            example: "- id: quality\n  uses: llm/score\n  with:\n    provider: openai\n    model: gpt-4o-mini\n    api_key: $inputs.scoring_key\n    rubric: Score how clearly the response explains its answer.\n    observations: [$steps.answer.outputs.stdout]\n    range: { min: 0, max: 10 }",
        }
    }

    async fn invoke(
        &self,
        _ctx: &ActionCtx<'_>,
        with: &serde_yml::Value,
    ) -> Result<ActionResult, ActionError> {
        let args: With =
            serde_yml::from_value(with.clone()).map_err(|source| ActionError::InvalidWith {
                action: USES,
                source,
            })?;
        let (provider, default_base, path) = args.provider.route();
        let mut result = ActionResult::error().with_output("provider", json!(provider));
        if !args.range.min.is_finite()
            || !args.range.max.is_finite()
            || args.range.min >= args.range.max
        {
            return Ok(result.with_output(
                "missing_cause",
                json!("score range must have finite bounds with min < max"),
            ));
        }
        if args.api_key.trim().is_empty() {
            return Ok(result.with_output("missing_cause", json!("provider API key is missing")));
        }
        let prompt = json!({
            "instruction": "Apply the rubric to the observations. Reply with one JSON object containing a numeric score within the inclusive range and a string reason.",
            "rubric": args.rubric,
            "observations": args.observations,
            "range": {"min": args.range.min, "max": args.range.max},
        }).to_string();
        let base = args
            .base_url
            .as_deref()
            .unwrap_or(default_base)
            .trim_end_matches('/');
        let endpoint = format!("{base}/{path}");
        let timeout = args.timeout.map_or(DEFAULT_TIMEOUT, Duration::from);
        result = result
            .with_output(
                "prompt_hash",
                json!(format!("{:x}", Sha256::digest(prompt.as_bytes()))),
            )
            .with_output(
                "request_params",
                json!({
                    "model": args.model, "temperature": 0, "max_tokens": MAX_TOKENS,
                    "endpoint": endpoint, "timeout_ms": timeout.as_millis(),
                    "range": {"min": args.range.min, "max": args.range.max},
                }),
            );
        let client = match reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(timeout)
            .build()
        {
            Ok(client) => client,
            Err(error) => return Ok(transport_error(result, error)),
        };
        let request = client.post(endpoint).json(&json!({
            "model": args.model, "max_tokens": MAX_TOKENS, "temperature": 0,
            "messages": [{"role": "user", "content": prompt}],
        }));
        let request = match args.provider {
            Provider::Anthropic => request
                .header("x-api-key", args.api_key)
                .header("anthropic-version", "2023-06-01"),
            Provider::Openai => request.bearer_auth(args.api_key),
        };
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => return Ok(transport_error(result, error)),
        };
        let status = response.status();
        let raw = match response.text().await {
            Ok(raw) => raw,
            Err(error) => return Ok(transport_error(result, error)),
        };
        let response = serde_json::from_str::<Value>(&raw);
        result = result.with_output("raw_response", json!(raw));
        if let Ok(response) = &response
            && let Some(model) = response.get("model").and_then(Value::as_str)
        {
            result = result.with_output("model", json!(model));
        }
        if !status.is_success() {
            return Ok(result.with_output(
                "missing_cause",
                json!(format!("provider returned HTTP {status}")),
            ));
        }
        let reply = response
            .ok()
            .as_ref()
            .and_then(|body| args.provider.content(body))
            .and_then(|text| serde_json::from_str::<Reply>(text).ok());
        let Some(reply) = reply else {
            return Ok(result.with_output(
                "missing_cause",
                json!("provider did not return JSON with numeric score and string reason"),
            ));
        };
        if !(args.range.min..=args.range.max).contains(&reply.score) {
            return Ok(result.with_output(
                "missing_cause",
                json!("provider score was outside the declared range"),
            ));
        }
        result.outcome = crate::Outcome::Ok;
        Ok(result
            .with_output("score", json!(reply.score))
            .with_output("reason", json!(reply.reason)))
    }
}
