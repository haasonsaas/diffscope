use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub model_name: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub temperature: f32,
    pub max_tokens: usize,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            model_name: "gpt-4o".to_string(),
            api_key: None,
            base_url: None,
            temperature: 0.2,
            max_tokens: 4000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMRequest {
    pub system_prompt: String,
    pub user_prompt: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

#[async_trait]
pub trait LLMAdapter: Send + Sync {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse>;
    fn model_name(&self) -> &str;
}

pub fn create_adapter(config: &ModelConfig) -> Result<Box<dyn LLMAdapter>> {
    match config.model_name.as_str() {
        // Anthropic Claude models (all versions)
        name if name.starts_with("claude-") => {
            Ok(Box::new(crate::adapters::AnthropicAdapter::new(config.clone())?))
        }
        // Legacy claude naming without dash
        name if name.starts_with("claude") => {
            Ok(Box::new(crate::adapters::AnthropicAdapter::new(config.clone())?))
        }
        // OpenAI models
        name if name.starts_with("gpt-") => {
            Ok(Box::new(crate::adapters::OpenAIAdapter::new(config.clone())?))
        }
        name if name.starts_with("o1-") => {
            Ok(Box::new(crate::adapters::OpenAIAdapter::new(config.clone())?))
        }
        // Ollama models
        name if name.starts_with("ollama:") => {
            Ok(Box::new(crate::adapters::OllamaAdapter::new(config.clone())?))
        }
        _name if config.base_url.as_ref().map_or(false, |u| u.contains("11434")) => {
            Ok(Box::new(crate::adapters::OllamaAdapter::new(config.clone())?))
        }
        // Default to OpenAI for unknown models
        _ => {
            Ok(Box::new(crate::adapters::OpenAIAdapter::new(config.clone())?))
        }
    }
}