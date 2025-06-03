use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use crate::adapters::llm::{LLMAdapter, LLMRequest, LLMResponse, ModelConfig, Usage};

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
        
        let base_url = config.base_url.clone()
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
}

#[async_trait]
impl LLMAdapter for AnthropicAdapter {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        let messages = vec![
            Message {
                role: "user".to_string(),
                content: request.user_prompt,
            },
        ];
        
        let anthropic_request = AnthropicRequest {
            model: self.config.model_name.clone(),
            messages,
            max_tokens: request.max_tokens.unwrap_or(self.config.max_tokens),
            temperature: request.temperature.unwrap_or(self.config.temperature),
            system: request.system_prompt,
        };
        
        let response = self.client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&anthropic_request)
            .send()
            .await
            .context("Failed to send request to Anthropic")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("Anthropic API error: {}", error_text);
        }
        
        let anthropic_response: AnthropicResponse = response.json().await
            .context("Failed to parse Anthropic response")?;
        
        let content = anthropic_response.content
            .first()
            .map(|c| c.text.clone())
            .unwrap_or_default();
        
        Ok(LLMResponse {
            content,
            model: anthropic_response.model,
            usage: Some(Usage {
                prompt_tokens: anthropic_response.usage.input_tokens,
                completion_tokens: anthropic_response.usage.output_tokens,
                total_tokens: anthropic_response.usage.input_tokens + anthropic_response.usage.output_tokens,
            }),
        })
    }
    
    fn model_name(&self) -> &str {
        &self.config.model_name
    }
}