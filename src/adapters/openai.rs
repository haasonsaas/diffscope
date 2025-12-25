use crate::adapters::llm::{LLMAdapter, LLMRequest, LLMResponse, ModelConfig, Usage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

pub struct OpenAIAdapter {
    client: Client,
    config: ModelConfig,
    api_key: String,
    base_url: String,
}

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: usize,
}

#[derive(Serialize)]
struct OpenAIResponsesRequest {
    model: String,
    input: String,
    instructions: String,
    temperature: f32,
    max_output_tokens: usize,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<Choice>,
    usage: OpenAIUsage,
    model: String,
}

#[derive(Deserialize)]
struct OpenAIResponsesResponse {
    output: Vec<OpenAIResponseOutput>,
    model: String,
    #[serde(default)]
    usage: Option<OpenAIResponsesUsage>,
}

#[derive(Deserialize)]
struct OpenAIResponseOutput {
    #[serde(rename = "type")]
    output_type: String,
    #[serde(default)]
    content: Vec<OpenAIResponseContent>,
}

#[derive(Deserialize)]
struct OpenAIResponseContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    prompt_tokens: usize,
    completion_tokens: usize,
    total_tokens: usize,
}

#[derive(Deserialize)]
struct OpenAIResponsesUsage {
    input_tokens: usize,
    output_tokens: usize,
    total_tokens: usize,
}

impl OpenAIAdapter {
    pub fn new(config: ModelConfig) -> Result<Self> {
        let api_key = config.api_key.clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .context("OpenAI API key not found. Set OPENAI_API_KEY environment variable or provide in config")?;

        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

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

                    anyhow::bail!("OpenAI API error ({}): {}", status, body);
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

        anyhow::bail!("OpenAI request failed after retries");
    }
}

#[async_trait]
impl LLMAdapter for OpenAIAdapter {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        if should_use_responses_api(&self.config) {
            return self.complete_responses(request).await;
        }

        self.complete_chat_completions(request).await
    }

    fn _model_name(&self) -> &str {
        &self.config.model_name
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn should_use_responses_api(config: &ModelConfig) -> bool {
    if let Some(flag) = config.openai_use_responses {
        return flag;
    }

    if let Some(base_url) = config.base_url.as_ref() {
        if !base_url.contains("openai.com") {
            return false;
        }
    }

    !config.model_name.starts_with("gpt-3.5")
}

impl OpenAIAdapter {
    async fn complete_chat_completions(&self, request: LLMRequest) -> Result<LLMResponse> {
        let messages = vec![
            Message {
                role: "system".to_string(),
                content: request.system_prompt,
            },
            Message {
                role: "user".to_string(),
                content: request.user_prompt,
            },
        ];

        let openai_request = OpenAIRequest {
            model: self.config.model_name.clone(),
            messages,
            temperature: request.temperature.unwrap_or(self.config.temperature),
            max_tokens: request.max_tokens.unwrap_or(self.config.max_tokens),
        };

        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .send_with_retry(|| {
                self.client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("Content-Type", "application/json")
                    .json(&openai_request)
            })
            .await
            .context("Failed to send request to OpenAI")?;

        let openai_response: OpenAIResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI response")?;

        let content = openai_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(LLMResponse {
            content,
            model: openai_response.model,
            usage: Some(Usage {
                prompt_tokens: openai_response.usage.prompt_tokens,
                completion_tokens: openai_response.usage.completion_tokens,
                total_tokens: openai_response.usage.total_tokens,
            }),
        })
    }

    async fn complete_responses(&self, request: LLMRequest) -> Result<LLMResponse> {
        let openai_request = OpenAIResponsesRequest {
            model: self.config.model_name.clone(),
            input: request.user_prompt,
            instructions: request.system_prompt,
            temperature: request.temperature.unwrap_or(self.config.temperature),
            max_output_tokens: request.max_tokens.unwrap_or(self.config.max_tokens),
        };

        let url = format!("{}/responses", self.base_url);
        let response = self
            .send_with_retry(|| {
                self.client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("Content-Type", "application/json")
                    .json(&openai_request)
            })
            .await
            .context("Failed to send request to OpenAI")?;

        let openai_response: OpenAIResponsesResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI response")?;

        let content = extract_response_text(&openai_response);
        let usage = openai_response.usage.map(|usage| Usage {
            prompt_tokens: usage.input_tokens,
            completion_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
        });

        Ok(LLMResponse {
            content,
            model: openai_response.model,
            usage,
        })
    }
}

fn extract_response_text(response: &OpenAIResponsesResponse) -> String {
    let mut combined = String::new();

    for item in &response.output {
        if item.output_type != "message" {
            continue;
        }
        for content in &item.content {
            if content.content_type == "output_text" {
                if let Some(text) = &content.text {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(text);
                }
            }
        }
    }

    combined
}
