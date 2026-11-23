//! A single provider call that records scoring evidence.
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{Action, ActionContract, ActionCtx, ActionError, ActionResult, FieldSpec, TimeoutSpec};

pub struct Score;

const USES: &str = duhem_schema::score::ACTION;
const MAX_TOKENS: u32 = 1024;
const DEFAULT_SCORE_TIMEOUT: Duration = Duration::from_secs(60);

fn effective_timeout(timeout: Option<TimeoutSpec>) -> Duration {
    timeout.map_or(DEFAULT_SCORE_TIMEOUT, Duration::from)
}

fn request_params(model: &str, endpoint: &str, timeout: Duration, range: &Range) -> Value {
    json!({
        "model": model, "temperature": 0, "max_tokens": MAX_TOKENS,
        "endpoint": endpoint, "timeout_ms": timeout.as_millis(),
        "range": {"min": range.min, "max": range.max},
    })
}

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

    fn content(self, response: &Value) -> Result<&str, &'static str> {
        let text = match self {
            Self::Anthropic => response
                .get("content")
                .and_then(Value::as_array)
                .and_then(|blocks| {
                    blocks
                        .iter()
                        .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                })
                .and_then(|block| block.get("text"))
                .and_then(Value::as_str)
                .ok_or("provider reply had no text content block")?,
            Self::Openai => response
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .ok_or("provider reply had no text content")?,
        };
        if text.trim().is_empty() {
            return Err("provider reply had empty text content");
        }
        Ok(text)
    }
}

fn parse_reply(text: &str) -> Result<Reply, &'static str> {
    if let Ok(reply) = serde_json::from_str::<Reply>(text) {
        return Ok(reply);
    }

    let mut objects = Vec::new();
    let bytes = text.as_bytes();
    let mut offset = 0;
    while let Some(start) = bytes[offset..].iter().position(|byte| *byte == b'{') {
        let start = offset + start;
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        let mut end = None;
        for (index, byte) in bytes.iter().enumerate().skip(start) {
            if in_string {
                if escaped {
                    escaped = false;
                } else if *byte == b'\\' {
                    escaped = true;
                } else if *byte == b'"' {
                    in_string = false;
                }
                continue;
            }
            match *byte {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(index + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            break;
        };
        if let Ok(Value::Object(object)) = serde_json::from_slice(&bytes[start..end]) {
            objects.push(Value::Object(object));
        }
        offset = end;
    }

    match objects.len() {
        0 => Err("provider reply contained no JSON object"),
        1 => serde_json::from_value::<Reply>(objects.remove(0))
            .map_err(|_| "provider did not return JSON with numeric score and string reason"),
        _ => Err("provider reply contained multiple JSON objects"),
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
            summary: "Record a rubric score in the non-verdict `scores` report and in step evidence. The report keys each score by criterion, check, and step ID. Requests default to a 60s timeout; set `timeout:` to override it. Replies may be strict JSON, one fenced JSON object, or prose containing one JSON object; multiple objects are rejected.",
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
        let timeout = effective_timeout(args.timeout);
        result = result
            .with_output(
                "prompt_hash",
                json!(format!("{:x}", Sha256::digest(prompt.as_bytes()))),
            )
            .with_output(
                "request_params",
                request_params(&args.model, &endpoint, timeout, &args.range),
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
            .ok_or("provider response was not valid JSON")
            .and_then(|body| args.provider.content(body))
            .and_then(parse_reply);
        let reply = match reply {
            Ok(reply) => reply,
            Err(cause) => return Ok(result.with_output("missing_cause", json!(cause))),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn recorded_timeout_ms(timeout: Option<TimeoutSpec>) -> u128 {
        request_params(
            "model",
            "https://example.test",
            effective_timeout(timeout),
            &Range {
                min: 0.0,
                max: 10.0,
            },
        )["timeout_ms"]
            .as_u64()
            .expect("timeout_ms") as u128
    }

    #[test]
    fn defaults_to_60_second_timeout() {
        assert_eq!(recorded_timeout_ms(None), 60_000);
    }

    #[test]
    fn explicit_timeout_overrides_default() {
        assert_eq!(
            recorded_timeout_ms(Some(TimeoutSpec(Duration::from_secs(2)))),
            2_000
        );
    }

    #[test]
    fn parses_plain_json() {
        let reply = parse_reply(r#"{"score":8.5,"reason":"clear"}"#).unwrap();
        assert_eq!(reply.score, 8.5);
        assert_eq!(reply.reason, "clear");
    }

    #[test]
    fn parses_json_in_a_code_fence() {
        let reply = parse_reply("```json\n{\"score\":8,\"reason\":\"clear\"}\n```").unwrap();
        assert_eq!(reply.score, 8.0);
        assert_eq!(reply.reason, "clear");
    }

    #[test]
    fn parses_one_json_object_embedded_in_prose() {
        let reply = parse_reply("Result: {\"score\":7,\"reason\":\"clear\"} Thanks.").unwrap();
        assert_eq!(reply.score, 7.0);
        assert_eq!(reply.reason, "clear");
    }

    #[test]
    fn reports_prose_without_a_json_object() {
        assert_eq!(
            parse_reply("There is no structured score here.").unwrap_err(),
            "provider reply contained no JSON object"
        );
    }

    #[test]
    fn rejects_multiple_json_objects() {
        assert_eq!(
            parse_reply(
                "First {\"score\":7,\"reason\":\"one\"}; second {\"score\":8,\"reason\":\"two\"}"
            )
            .unwrap_err(),
            "provider reply contained multiple JSON objects"
        );
    }

    #[test]
    fn anthropic_without_text_block_has_a_specific_cause() {
        let response = json!({"content": [{"type": "thinking"}]});
        assert_eq!(
            Provider::Anthropic.content(&response),
            Err("provider reply had no text content block")
        );
    }

    #[test]
    fn empty_anthropic_text_content_has_a_specific_cause() {
        let response = json!({"content": [{"type": "text", "text": "  "}]});
        assert_eq!(
            Provider::Anthropic.content(&response),
            Err("provider reply had empty text content")
        );
    }

    #[test]
    fn openai_text_uses_the_shared_reply_parser() {
        let response = json!({
            "choices": [{"message": {"content": "```\n{\"score\":9,\"reason\":\"clear\"}\n```"}}]
        });
        let reply = parse_reply(Provider::Openai.content(&response).unwrap()).unwrap();
        assert_eq!(reply.score, 9.0);
        assert_eq!(reply.reason, "clear");
    }
}
