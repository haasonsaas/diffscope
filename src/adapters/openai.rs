use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use crate::adapters::llm::{LLMAdapter, LLMRequest, LLMResponse, ModelConfig, Usage};

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
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    prompt_tokens: usize,
    completion_tokens: usize,
    total_tokens: usize,
}

impl OpenAIAdapter {
    pub fn new(config: ModelConfig) -> Result<Self> {
        let api_key = config.api_key.clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .context("OpenAI API key not found. Set OPENAI_API_KEY environment variable or provide in config")?;
        
        let base_url = config.base_url.clone()
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
}

#[async_trait]
impl LLMAdapter for OpenAIAdapter {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
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
        
        let response = self.client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&openai_request)
            .send()
            .await
            .context("Failed to send request to OpenAI")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("OpenAI API error: {}", error_text);
        }
        
        let openai_response: OpenAIResponse = response.json().await
            .context("Failed to parse OpenAI response")?;
        
        let content = openai_response.choices
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
    
    fn _model_name(&self) -> &str {
        &self.config.model_name
    }
}