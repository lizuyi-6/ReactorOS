use anyhow::{anyhow, Context, Result};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    config::OptimizerBounds, db::BatchOutcome, memory::AiMemory, number::round2,
    optimizer::Recommendation,
};

const DEFAULT_BASE_URL: &str = "https://api.stepfun.com/v1";
const DEFAULT_MODEL: &str = "step-3.6";
const DEFAULT_REASONING_EFFORT: &str = "medium";
const DEFAULT_API_TYPE: StepFunApiType = StepFunApiType::ChatCompletions;
const STEPFUN_MAX_ATTEMPTS: usize = 3;
const STEPFUN_RETRY_BASE_MS: u64 = 300;
const STEPFUN_RETRY_MAX_MS: u64 = 5_000;

#[derive(Clone)]
pub struct AiProvider {
    client: Client,
    config: AiProviderConfig,
}

#[derive(Debug, Clone)]
pub struct AiProviderConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: String,
    pub api_type: StepFunApiType,
    pub model: String,
    pub reasoning_effort: String,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepFunApiType {
    ChatCompletions,
    Messages,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRecommendationEnvelope {
    #[serde(flatten)]
    pub recommendation: Recommendation,
    pub provider: AiRecommendationProvider,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRecommendationProvider {
    pub mode: String,
    pub model: String,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    reasoning: String,
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<MessageContentBlock>,
}

#[derive(Debug, Deserialize)]
struct MessageContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct ModelRecommendation {
    target_temperature_c: f64,
    target_stirrer_rpm: f64,
    heating_minutes: f64,
    stirring_minutes: f64,
    expected_score: f64,
    rationale: String,
}

impl AiProvider {
    pub fn from_env() -> Result<Option<Self>> {
        let config = AiProviderConfig::from_env();
        Self::from_config(config)
    }

    pub fn from_config(config: AiProviderConfig) -> Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }
        if config
            .api_key
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err(anyhow!(
                "STEPFUN_AI_ENABLED is true but STEPFUN_API_KEY is not set"
            ));
        }
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_seconds))
            .build()
            .context("failed to build StepFun HTTP client")?;
        Ok(Some(Self { client, config }))
    }

    pub fn model_name(&self) -> &str {
        &self.config.model
    }

    pub async fn recommend(
        &self,
        bounds: &OptimizerBounds,
        memory: &AiMemory,
        outcomes: &[BatchOutcome],
        fallback: &Recommendation,
    ) -> Result<Recommendation> {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("StepFun API key is not configured"))?;
        let endpoint = self.endpoint();
        let request = build_request(&self.config, bounds, memory, outcomes, fallback)?;

        let mut last_error = None;
        for attempt in 1..=STEPFUN_MAX_ATTEMPTS {
            let response = self
                .client
                .post(endpoint.clone())
                .bearer_auth(api_key)
                .json(&request)
                .send()
                .await;

            let response = match response {
                Ok(response) => response,
                Err(err) => {
                    let retryable = err.is_timeout() || err.is_connect() || err.is_request();
                    let message = anyhow!(
                        "StepFun recommendation request failed on attempt {attempt}/{STEPFUN_MAX_ATTEMPTS}: {err}"
                    );
                    if retryable && attempt < STEPFUN_MAX_ATTEMPTS {
                        last_error = Some(message);
                        tokio::time::sleep(stepfun_retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(message);
                }
            };

            let status = response.status();
            let text = response
                .text()
                .await
                .context("failed to read StepFun response body")?;
            if !status.is_success() {
                let message = anyhow!("StepFun returned HTTP {status}: {text}");
                if retryable_status(status) && attempt < STEPFUN_MAX_ATTEMPTS {
                    last_error = Some(message);
                    tokio::time::sleep(stepfun_retry_delay(attempt)).await;
                    continue;
                }
                return Err(message);
            }
            let content = extract_response_content(self.config.api_type, &text)?;
            let model_rec: ModelRecommendation =
                serde_json::from_str(&content).or_else(|_| parse_json_block(&content))?;
            return validate_model_recommendation(model_rec, bounds, memory, outcomes, fallback);
        }
        Err(last_error
            .unwrap_or_else(|| anyhow!("StepFun recommendation request failed after retries")))
    }

    fn endpoint(&self) -> String {
        endpoint_url(&self.config.base_url, self.config.api_type)
    }
}

impl AiProviderConfig {
    fn from_env() -> Self {
        Self {
            enabled: env_bool("STEPFUN_AI_ENABLED"),
            api_key: std::env::var("STEPFUN_API_KEY").ok(),
            base_url: std::env::var("STEPFUN_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string()),
            api_type: std::env::var("STEPFUN_API_TYPE")
                .ok()
                .and_then(|value| StepFunApiType::parse(&value))
                .unwrap_or(DEFAULT_API_TYPE),
            model: std::env::var("STEPFUN_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
            reasoning_effort: std::env::var("STEPFUN_REASONING_EFFORT")
                .unwrap_or_else(|_| DEFAULT_REASONING_EFFORT.to_string()),
            timeout_seconds: std::env::var("STEPFUN_TIMEOUT_SECONDS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(20),
        }
    }
}

impl StepFunApiType {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "chat" | "chat_completion" | "chat_completions" | "chat-completions" => {
                Some(Self::ChatCompletions)
            }
            "message" | "messages" | "anthropic" => Some(Self::Messages),
            _ => None,
        }
    }
}

pub fn local_envelope(recommendation: Recommendation) -> AiRecommendationEnvelope {
    AiRecommendationEnvelope {
        recommendation,
        provider: AiRecommendationProvider {
            mode: "local_optimizer".to_string(),
            model: "local-ga-sa-pid".to_string(),
            fallback_reason: None,
        },
    }
}

pub fn fallback_envelope(
    recommendation: Recommendation,
    model: impl Into<String>,
    reason: impl Into<String>,
) -> AiRecommendationEnvelope {
    AiRecommendationEnvelope {
        recommendation,
        provider: AiRecommendationProvider {
            mode: "fallback_local_optimizer".to_string(),
            model: model.into(),
            fallback_reason: Some(reason.into()),
        },
    }
}

pub fn stale_local_envelope(
    recommendation: Recommendation,
    model: impl Into<String>,
    reason: impl Into<String>,
) -> AiRecommendationEnvelope {
    AiRecommendationEnvelope {
        recommendation,
        provider: AiRecommendationProvider {
            mode: "stale_local_recommendation".to_string(),
            model: model.into(),
            fallback_reason: Some(reason.into()),
        },
    }
}

pub fn stepfun_envelope(
    recommendation: Recommendation,
    model: impl Into<String>,
) -> AiRecommendationEnvelope {
    AiRecommendationEnvelope {
        recommendation,
        provider: AiRecommendationProvider {
            mode: "stepfun".to_string(),
            model: model.into(),
            fallback_reason: None,
        },
    }
}

fn build_request(
    config: &AiProviderConfig,
    bounds: &OptimizerBounds,
    memory: &AiMemory,
    outcomes: &[BatchOutcome],
    fallback: &Recommendation,
) -> Result<serde_json::Value> {
    let user_content = format!(
        "{}\n\n{}",
        "Return one valid minified JSON object only. Do not wrap it in markdown. Do not include comments.",
        serde_json::to_string_pretty(&build_model_context(bounds, memory, outcomes, fallback))?
    );

    let request = match config.api_type {
        StepFunApiType::ChatCompletions => json!({
            "model": config.model,
            "messages": [
                {
                    "role": "system",
                    "content": system_prompt()
                },
                {
                    "role": "user",
                    "content": user_content
                }
            ],
            "reasoning_effort": config.reasoning_effort,
            "max_tokens": 4096,
            "temperature": 0.1,
            "response_format": { "type": "json_object" }
        }),
        StepFunApiType::Messages => json!({
            "model": config.model,
            "max_tokens": 4096,
            "system": system_prompt(),
            "messages": [
                {
                    "role": "user",
                    "content": user_content
                }
            ],
            "temperature": 0.1
        }),
    };

    Ok(request)
}

fn endpoint_url(base_url: &str, api_type: StepFunApiType) -> String {
    let mut base = base_url.trim().trim_end_matches('/').to_string();
    let expected_path = match api_type {
        StepFunApiType::ChatCompletions => "/v1/chat/completions",
        StepFunApiType::Messages => "/v1/messages",
    };
    if base.ends_with(expected_path) {
        return base;
    }
    if !base.ends_with("/v1") {
        base.push_str("/v1");
    }
    match api_type {
        StepFunApiType::ChatCompletions => format!("{base}/chat/completions"),
        StepFunApiType::Messages => format!("{base}/messages"),
    }
}

fn retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn stepfun_retry_delay(attempt: usize) -> std::time::Duration {
    let multiplier = 1_u64 << attempt.saturating_sub(1).min(16);
    std::time::Duration::from_millis((STEPFUN_RETRY_BASE_MS * multiplier).min(STEPFUN_RETRY_MAX_MS))
}

fn extract_response_content(api_type: StepFunApiType, text: &str) -> Result<String> {
    match api_type {
        StepFunApiType::ChatCompletions => extract_chat_completion_content(text),
        StepFunApiType::Messages => extract_messages_content(text),
    }
}

fn extract_chat_completion_content(text: &str) -> Result<String> {
    let response: ChatCompletionResponse =
        serde_json::from_str(text).context("failed to parse StepFun chat response")?;
    response
        .choices
        .first()
        .map(|choice| {
            if choice.message.content.trim().is_empty() {
                choice.message.reasoning.trim()
            } else {
                choice.message.content.trim()
            }
        })
        .filter(|content| !content.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("StepFun response did not contain message content or reasoning"))
}

fn extract_messages_content(text: &str) -> Result<String> {
    let response: MessagesResponse =
        serde_json::from_str(text).context("failed to parse StepFun messages response")?;
    let content = response
        .content
        .iter()
        .filter(|block| block.block_type == "text")
        .map(|block| block.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if content.trim().is_empty() {
        Err(anyhow!(
            "StepFun messages response did not contain text content"
        ))
    } else {
        Ok(content)
    }
}

fn build_model_context(
    bounds: &OptimizerBounds,
    memory: &AiMemory,
    outcomes: &[BatchOutcome],
    fallback: &Recommendation,
) -> serde_json::Value {
    let effective_bounds = memory.effective_optimizer_bounds(bounds);
    let recent_outcomes: Vec<_> = outcomes.iter().rev().take(12).cloned().collect();
    json!({
        "task": "Recommend next batch reactor parameters.",
        "hard_rules": [
            "Return JSON only, no markdown.",
            "Never exceed bounds.",
            "Never choose a point inside forbidden_zones.",
            "This is advisory only; downstream safety logic will re-check all values."
        ],
        "bounds": effective_bounds,
        "objective": {
            "optimize_for": memory.objective.optimize_for,
            "yield_weight": memory.objective.yield_weight,
            "product_ratio_weight": memory.objective.product_ratio_weight,
            "notes": memory.objective.notes
        },
        "profile": memory.profile,
        "reference_batches": memory.reference_batches,
        "forbidden_zones": memory.forbidden_zones,
        "recent_finished_batches": recent_outcomes,
        "local_optimizer_reference": fallback,
        "required_schema": {
            "target_temperature_c": "number",
            "target_stirrer_rpm": "number",
            "heating_minutes": "number",
            "stirring_minutes": "number",
            "expected_score": "number from 0 to 100",
            "rationale": "short Chinese reason for operator, <= 120 chars"
        }
    })
}

fn system_prompt() -> &'static str {
    "You are an industrial lab reactor optimization assistant. Recommend only batch-level setpoints, not direct actuator commands. Respect all provided safety bounds and forbidden zones. Your entire response must be valid JSON only, starting with { and ending with }. No markdown."
}

fn validate_model_recommendation(
    model: ModelRecommendation,
    bounds: &OptimizerBounds,
    memory: &AiMemory,
    outcomes: &[BatchOutcome],
    fallback: &Recommendation,
) -> Result<Recommendation> {
    let effective_bounds = memory.effective_optimizer_bounds(bounds);
    let target_temperature_c = finite_in_range(
        "target_temperature_c",
        model.target_temperature_c,
        effective_bounds.min_temperature_c,
        effective_bounds.max_temperature_c,
    )?;
    let target_stirrer_rpm = finite_in_range(
        "target_stirrer_rpm",
        model.target_stirrer_rpm,
        effective_bounds.min_stirrer_rpm,
        effective_bounds.max_stirrer_rpm,
    )?;
    let heating_minutes = finite_in_range(
        "heating_minutes",
        model.heating_minutes,
        effective_bounds.min_heating_minutes,
        effective_bounds.max_heating_minutes,
    )?;
    let stirring_minutes = finite_in_range(
        "stirring_minutes",
        model.stirring_minutes,
        effective_bounds.min_stirring_minutes,
        effective_bounds.max_stirring_minutes,
    )?;

    let mut recommendation = Recommendation {
        based_on_batch_count: outcomes.len() as i64,
        target_temperature_c: round2(target_temperature_c),
        target_stirrer_rpm: round2(target_stirrer_rpm),
        heating_minutes: round2(heating_minutes),
        stirring_minutes: round2(stirring_minutes),
        expected_score: round2(clamp_finite(
            model.expected_score,
            0.0,
            100.0,
            fallback.expected_score,
        )),
        rationale: sanitize_rationale(model.rationale, &fallback.rationale),
    };

    if memory.forbidden_zones.iter().any(|zone| {
        zone.contains(
            recommendation.target_temperature_c,
            recommendation.target_stirrer_rpm,
            recommendation.heating_minutes,
            recommendation.stirring_minutes,
        )
    }) {
        return Err(anyhow!(
            "StepFun recommendation was rejected by forbidden-zone validation; local fallback is disabled when StepFun is configured"
        ));
    } else if recommendation.rationale.is_empty() {
        recommendation.rationale = "StepFun 根据历史批次、参考记忆和禁区生成推荐。".to_string();
    } else {
        recommendation.rationale = format!("StepFun: {}", recommendation.rationale);
    }

    Ok(recommendation)
}

fn parse_json_block<T: for<'de> Deserialize<'de>>(content: &str) -> Result<T> {
    let content = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let start = content
        .find('{')
        .ok_or_else(|| anyhow!("StepFun content did not contain JSON object"))?;
    let end = content
        .rfind('}')
        .ok_or_else(|| anyhow!("StepFun content did not contain JSON object end"))?;
    serde_json::from_str(&content[start..=end]).context("failed to parse JSON object from content")
}

fn clamp_finite(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback.clamp(min, max)
    }
}

fn finite_in_range(field: &str, value: f64, min: f64, max: f64) -> Result<f64> {
    if !value.is_finite() {
        return Err(anyhow!("StepFun field {field} must be finite"));
    }
    if !(min..=max).contains(&value) {
        return Err(anyhow!(
            "StepFun field {field}={value} is outside safety bounds {min}..{max}; local fallback is disabled"
        ));
    }
    Ok(value)
}

fn sanitize_rationale(value: String, fallback: &str) -> String {
    let mut value = value.trim().replace(['\r', '\n'], " ");
    if value.len() > 180 {
        value.truncate(180);
    }
    if value.is_empty() {
        fallback.chars().take(120).collect()
    } else {
        value
    }
}

fn env_bool(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_url_accepts_base_url_with_or_without_v1() {
        assert_eq!(
            endpoint_url(
                "https://api.stepfun.com/v1",
                StepFunApiType::ChatCompletions
            ),
            "https://api.stepfun.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("https://api.stepfun.com/", StepFunApiType::Messages),
            "https://api.stepfun.com/v1/messages"
        );
        assert_eq!(
            endpoint_url(
                "https://api.stepfun.com/v1/chat/completions",
                StepFunApiType::ChatCompletions
            ),
            "https://api.stepfun.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url(
                "https://api.stepfun.com/v1/messages",
                StepFunApiType::Messages
            ),
            "https://api.stepfun.com/v1/messages"
        );
    }

    #[test]
    fn parses_chat_completion_content() {
        let text = r#"{
            "choices": [
                {
                    "message": {
                        "content": "{\"target_temperature_c\":120.0}"
                    }
                }
            ]
        }"#;
        assert_eq!(
            extract_response_content(StepFunApiType::ChatCompletions, text).unwrap(),
            r#"{"target_temperature_c":120.0}"#
        );
    }

    #[test]
    fn parses_messages_content_blocks() {
        let text = r#"{
            "content": [
                {
                    "type": "text",
                    "text": "{\"target_temperature_c\":121.0}"
                }
            ]
        }"#;
        assert_eq!(
            extract_response_content(StepFunApiType::Messages, text).unwrap(),
            r#"{"target_temperature_c":121.0}"#
        );
    }

    #[test]
    fn stepfun_retry_delay_is_exponential_and_capped() {
        assert_eq!(
            stepfun_retry_delay(1),
            std::time::Duration::from_millis(300)
        );
        assert_eq!(
            stepfun_retry_delay(2),
            std::time::Duration::from_millis(600)
        );
        assert_eq!(
            stepfun_retry_delay(3),
            std::time::Duration::from_millis(1200)
        );
        assert_eq!(
            stepfun_retry_delay(9),
            std::time::Duration::from_millis(5000)
        );
    }
}
