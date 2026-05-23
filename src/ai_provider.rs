use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    config::OptimizerBounds, db::BatchOutcome, memory::AiMemory, optimizer::Recommendation,
};

const DEFAULT_BASE_URL: &str = "https://api.stepfun.com/v1";
const DEFAULT_MODEL: &str = "step-3.6";
const DEFAULT_REASONING_EFFORT: &str = "medium";

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
    pub model: String,
    pub reasoning_effort: String,
    pub timeout_seconds: u64,
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
            tracing::warn!("STEPFUN_AI_ENABLED is true but STEPFUN_API_KEY is not set");
            return Ok(None);
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
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let body = json!({
            "model": self.config.model,
            "messages": [
                {
                    "role": "system",
                    "content": system_prompt()
                },
                {
                    "role": "user",
                    "content": format!(
                        "{}\n\n{}",
                        "Return one valid minified JSON object only. Do not wrap it in markdown. Do not include comments.",
                        serde_json::to_string_pretty(&build_model_context(bounds, memory, outcomes, fallback))?
                    )
                }
            ],
            "reasoning_effort": self.config.reasoning_effort,
            "max_tokens": 4096,
            "temperature": 0.1,
            "response_format": { "type": "json_object" }
        });

        let response = self
            .client
            .post(endpoint)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .context("StepFun recommendation request failed")?;
        let status = response.status();
        let text = response
            .text()
            .await
            .context("failed to read StepFun response body")?;
        if !status.is_success() {
            return Err(anyhow!("StepFun returned HTTP {status}: {text}"));
        }
        let response: ChatCompletionResponse =
            serde_json::from_str(&text).context("failed to parse StepFun chat response")?;
        let content = response
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
            .ok_or_else(|| {
                anyhow!("StepFun response did not contain message content or reasoning")
            })?;
        let model_rec: ModelRecommendation =
            serde_json::from_str(content).or_else(|_| parse_json_block(content))?;
        Ok(validate_model_recommendation(
            model_rec, bounds, memory, outcomes, fallback,
        ))
    }
}

impl AiProviderConfig {
    fn from_env() -> Self {
        Self {
            enabled: env_bool("STEPFUN_AI_ENABLED"),
            api_key: std::env::var("STEPFUN_API_KEY").ok(),
            base_url: std::env::var("STEPFUN_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string()),
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

pub fn local_envelope(recommendation: Recommendation) -> AiRecommendationEnvelope {
    AiRecommendationEnvelope {
        recommendation,
        provider: AiRecommendationProvider {
            mode: "local_optimizer".to_string(),
            model: "local-tpe-lite".to_string(),
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

pub fn stepfun_envelope(
    recommendation: Recommendation,
    model: impl Into<String>,
) -> AiRecommendationEnvelope {
    AiRecommendationEnvelope {
        recommendation,
        provider: AiRecommendationProvider {
            mode: "stepfun_chat_completion".to_string(),
            model: model.into(),
            fallback_reason: None,
        },
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
        "local_optimizer_fallback": fallback,
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
) -> Recommendation {
    let effective_bounds = memory.effective_optimizer_bounds(bounds);
    let target_temperature_c = clamp_finite(
        model.target_temperature_c,
        effective_bounds.min_temperature_c,
        effective_bounds.max_temperature_c,
        fallback.target_temperature_c,
    );
    let target_stirrer_rpm = clamp_finite(
        model.target_stirrer_rpm,
        effective_bounds.min_stirrer_rpm,
        effective_bounds.max_stirrer_rpm,
        fallback.target_stirrer_rpm,
    );
    let heating_minutes = clamp_finite(
        model.heating_minutes,
        effective_bounds.min_heating_minutes,
        effective_bounds.max_heating_minutes,
        fallback.heating_minutes,
    );
    let stirring_minutes = clamp_finite(
        model.stirring_minutes,
        effective_bounds.min_stirring_minutes,
        effective_bounds.max_stirring_minutes,
        fallback.stirring_minutes,
    );

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
        recommendation = fallback.clone();
        recommendation.rationale = format!(
            "StepFun 推荐落入禁区，已回退本地优化器。{}",
            fallback.rationale
        );
    } else if recommendation.rationale.is_empty() {
        recommendation.rationale = "StepFun 根据历史批次、参考记忆和禁区生成推荐。".to_string();
    } else {
        recommendation.rationale = format!("StepFun: {}", recommendation.rationale);
    }

    recommendation
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

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn env_bool(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}
