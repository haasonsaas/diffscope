use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;
use crate::adapters::llm::{LLMAdapter, LLMRequest, LLMResponse, ModelConfig, Usage};

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
    _context: Option<Vec<i32>>,
    _total_duration: Option<u64>,
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

                    anyhow::bail!("Ollama API error ({}): {}", status, body);
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

        anyhow::bail!("Ollama request failed after retries");
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
        
        let url = format!("{}/api/generate", self.base_url);
        let response = self.send_with_retry(|| {
            self.client
                .post(&url)
                .json(&ollama_request)
        })
        .await
        .context("Failed to send request to Ollama")?;
        
        let ollama_response: OllamaResponse = response.json().await
            .context("Failed to parse Ollama response")?;
        
        Ok(LLMResponse {
            content: ollama_response.response,
            model: ollama_response.model,
            usage: if ollama_response.done {
                Some(Usage {
                    prompt_tokens: ollama_response.prompt_eval_count.unwrap_or(0),
                    completion_tokens: ollama_response.eval_count.unwrap_or(0),
                    total_tokens: ollama_response.prompt_eval_count.unwrap_or(0) + ollama_response.eval_count.unwrap_or(0),
                })
            } else {
                None
            },
        })
    }
    
    fn _model_name(&self) -> &str {
        &self.config.model_name
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}
