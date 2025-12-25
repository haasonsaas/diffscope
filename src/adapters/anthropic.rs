use crate::adapters::llm::{LLMAdapter, LLMRequest, LLMResponse, ModelConfig, Usage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

pub struct AnthropicAdapter {
    client: Client,
    config: ModelConfig,
    api_key: String,
    base_url: String,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<Message>,
    max_tokens: usize,
    temperature: f32,
    system: String,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<Content>,
    model: String,
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct Content {
    text: String,
    #[serde(rename = "type")]
    content_type: String,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: usize,
    output_tokens: usize,
}

impl AnthropicAdapter {
    pub fn new(config: ModelConfig) -> Result<Self> {
        let api_key = config.api_key.clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .context("Anthropic API key not found. Set ANTHROPIC_API_KEY environment variable or provide in config")?;

        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?;

        Ok(Self {
            client,
            config,
            api_key,
            base_url,
        })
    }

    async fn send_with_retry<F>(&self, mut make_request: F) -> Result<reqwest::Response>
    where
        F: FnMut() -> reqwest::RequestBuilder,
    {
        const MAX_RETRIES: usize = 2;
        const BASE_DELAY_MS: u64 = 250;

        for attempt in 0..=MAX_RETRIES {
            match make_request().send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        return Ok(response);
                    }

                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    if is_retryable_status(status) && attempt < MAX_RETRIES {
                        sleep(Duration::from_millis(BASE_DELAY_MS * (attempt as u64 + 1))).await;
                        continue;
                    }

                    anyhow::bail!("Anthropic API error ({}): {}", status, body);
                }
                Err(err) => {
                    if attempt < MAX_RETRIES {
                        sleep(Duration::from_millis(BASE_DELAY_MS * (attempt as u64 + 1))).await;
                        continue;
                    }
                    return Err(err.into());
                }
            }
        }

        anyhow::bail!("Anthropic request failed after retries");
    }
}

#[async_trait]
impl LLMAdapter for AnthropicAdapter {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        let messages = vec![Message {
            role: "user".to_string(),
            content: request.user_prompt,
        }];

        let anthropic_request = AnthropicRequest {
            model: self.config.model_name.clone(),
            messages,
            max_tokens: request.max_tokens.unwrap_or(self.config.max_tokens),
            temperature: request.temperature.unwrap_or(self.config.temperature),
            system: request.system_prompt,
        };

        let url = format!("{}/messages", self.base_url);
        let response = self
            .send_with_retry(|| {
                self.client
                    .post(&url)
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("anthropic-beta", "messages-2023-12-15")
                    .header("Content-Type", "application/json")
                    .json(&anthropic_request)
            })
            .await
            .context("Failed to send request to Anthropic")?;

        let anthropic_response: AnthropicResponse = response
            .json()
            .await
            .context("Failed to parse Anthropic response")?;

        let content = anthropic_response
            .content
            .first()
            .map(|c| {
                // Verify it's a text content type
                if c.content_type == "text" {
                    c.text.clone()
                } else {
                    format!("Unsupported content type: {}", c.content_type)
                }
            })
            .unwrap_or_default();

        Ok(LLMResponse {
            content,
            model: anthropic_response.model,
            usage: Some(Usage {
                prompt_tokens: anthropic_response.usage.input_tokens,
                completion_tokens: anthropic_response.usage.output_tokens,
                total_tokens: anthropic_response.usage.input_tokens
                    + anthropic_response.usage.output_tokens,
            }),
        })
    }

    fn _model_name(&self) -> &str {
        &self.config.model_name
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}
