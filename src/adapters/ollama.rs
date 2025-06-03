use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use crate::adapters::llm::{LLMAdapter, LLMRequest, LLMResponse, ModelConfig};

pub struct OllamaAdapter {
    client: Client,
    config: ModelConfig,
    base_url: String,
}

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    system: String,
    temperature: f32,
    num_predict: usize,
    stream: bool,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
    model: String,
    done: bool,
    context: Option<Vec<i32>>,
    total_duration: Option<u64>,
    prompt_eval_count: Option<usize>,
    eval_count: Option<usize>,
}

impl OllamaAdapter {
    pub fn new(config: ModelConfig) -> Result<Self> {
        let base_url = config.base_url.clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string());
        
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;
        
        Ok(Self {
            client,
            config,
            base_url,
        })
    }
}

#[async_trait]
impl LLMAdapter for OllamaAdapter {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        let model_name = self.config.model_name
            .strip_prefix("ollama:")
            .unwrap_or(&self.config.model_name);
        
        let ollama_request = OllamaRequest {
            model: model_name.to_string(),
            prompt: request.user_prompt,
            system: request.system_prompt,
            temperature: request.temperature.unwrap_or(self.config.temperature),
            num_predict: request.max_tokens.unwrap_or(self.config.max_tokens),
            stream: false,
        };
        
        let response = self.client
            .post(format!("{}/api/generate", self.base_url))
            .json(&ollama_request)
            .send()
            .await
            .context("Failed to send request to Ollama")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("Ollama API error: {}", error_text);
        }
        
        let ollama_response: OllamaResponse = response.json().await
            .context("Failed to parse Ollama response")?;
        
        Ok(LLMResponse {
            content: ollama_response.response,
            model: ollama_response.model,
            usage: None,
        })
    }
    
    fn model_name(&self) -> &str {
        &self.config.model_name
    }
}