use super::*;

pub(crate) async fn get_doctor(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let config = state.config.read().await.clone();
    let validation_issues = config.validation_issues();
    let role_providers = serde_json::json!({
        "primary": config.resolved_provider_for_role(crate::config::ModelRole::Primary),
        "weak": config.resolved_provider_for_role(crate::config::ModelRole::Weak),
        "reasoning": config.resolved_provider_for_role(crate::config::ModelRole::Reasoning),
        "embedding": config.resolved_provider_for_role(crate::config::ModelRole::Embedding),
        "fast": config.resolved_provider_for_role(crate::config::ModelRole::Fast),
    });
    let semantic_feedback_path = crate::core::default_semantic_feedback_path(&config.feedback_path);

    let base_url = config
        .base_url
        .clone()
        .unwrap_or_else(|| "http://localhost:11434".to_string());

    let mut result = serde_json::json!({
        "config": {
            "model": config.model,
            "adapter": config.adapter,
            "base_url": base_url.clone(),
            "api_key_set": config.api_key.is_some(),
            "context_window": config.context_window,
            "role_providers": role_providers,
        },
        "learning": {
            "enhanced_feedback": config.enhanced_feedback,
            "semantic_feedback": config.semantic_feedback,
            "semantic_rag": config.semantic_rag,
            "feedback_path": config.feedback_path.display().to_string(),
            "semantic_feedback_path": semantic_feedback_path.display().to_string(),
            "feedback_store_exists": config.feedback_path.exists(),
            "semantic_feedback_store_exists": semantic_feedback_path.exists(),
            "min_feedback_observations": config.feedback_min_observations,
            "semantic_feedback_min_examples": config.semantic_feedback_min_examples,
            "semantic_feedback_similarity": config.semantic_feedback_similarity,
        },
        "validation_issues": validation_issues,
        "endpoint_reachable": false,
        "endpoint_type": null,
        "models": [],
        "recommended_model": null,
    });

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Json(result),
    };

    // Check Ollama
    let ollama_url = format!("{base_url}/api/tags");
    if let Ok(resp) = client.get(&ollama_url).send().await {
        if resp.status().is_success() {
            result["endpoint_reachable"] = serde_json::json!(true);
            result["endpoint_type"] = serde_json::json!("ollama");
            if let Ok(body) = resp.text().await {
                if let Ok(models) =
                    crate::core::offline::OfflineModelManager::parse_model_list(&body)
                {
                    let model_names: Vec<serde_json::Value> = models
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "name": m.name,
                                "size_mb": m.size_mb,
                                "quantization": m.quantization,
                                "family": m.family,
                                "parameter_size": m.parameter_size,
                            })
                        })
                        .collect();
                    result["models"] = serde_json::json!(model_names);

                    let mut manager = crate::core::offline::OfflineModelManager::new(&base_url);
                    manager.set_models(models);
                    if let Some(rec) = manager.recommend_review_model() {
                        result["recommended_model"] = serde_json::json!(rec.name);
                    }
                }
            }
        }
    }

    // Check OpenAI-compatible
    if !result["endpoint_reachable"].as_bool().unwrap_or(false) {
        let openai_url = format!("{base_url}/v1/models");
        if let Ok(resp) = client.get(&openai_url).send().await {
            if resp.status().is_success() {
                result["endpoint_reachable"] = serde_json::json!(true);
                result["endpoint_type"] = serde_json::json!("openai-compatible");
            }
        }
    }

    Json(result)
}

pub(crate) async fn get_config(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let config = state.config.read().await;
    let mut value = serde_json::to_value(&*config).unwrap_or_default();
    if let Some(obj) = value.as_object_mut() {
        mask_config_secrets(obj);
    }
    Json(value)
}

pub(crate) async fn update_config(
    State(state): State<Arc<AppState>>,
    Json(updates): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut config = state.config.write().await;

    let mut current = serde_json::to_value(&*config).unwrap_or_default();
    if let (Some(current_obj), Some(updates_obj)) = (current.as_object_mut(), updates.as_object()) {
        for (key, value) in updates_obj {
            // Skip masked secret fields (don't overwrite with "***")
            if value.as_str() == Some("***") {
                continue;
            }
            current_obj.insert(key.clone(), value.clone());
        }
    }

    let new_config: crate::config::Config = serde_json::from_value(current)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid config: {e}")))?;

    *config = new_config;
    config.normalize();

    // Build response while still holding the write lock
    let mut result = serde_json::to_value(&*config).unwrap_or_default();
    if let Some(obj) = result.as_object_mut() {
        mask_config_secrets(obj);
    }

    drop(config);

    // Persist config to disk
    AppState::save_config_async(&state);

    Ok(Json(result))
}

/// Mask all secret fields in a config object for safe serialization.
pub(crate) fn mask_config_secrets(obj: &mut serde_json::Map<String, serde_json::Value>) {
    for key in &[
        "api_key",
        "github_token",
        "github_client_secret",
        "github_private_key",
        "github_webhook_secret",
        "jira_api_token",
        "linear_api_key",
        "automation_webhook_secret",
        "server_api_key",
        "vault_token",
    ] {
        if obj.get(*key).and_then(|v| v.as_str()).is_some() {
            obj.insert(key.to_string(), serde_json::json!("***"));
        }
    }
    mask_provider_api_keys(obj);
}

/// Mask api_key fields inside the providers map for safe serialization.
pub(crate) fn mask_provider_api_keys(obj: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(serde_json::Value::Object(providers)) = obj.get_mut("providers") {
        for (_name, provider_val) in providers.iter_mut() {
            if let serde_json::Value::Object(provider) = provider_val {
                if provider.get("api_key").and_then(|v| v.as_str()).is_some() {
                    provider.insert("api_key".to_string(), serde_json::json!("***"));
                }
            }
        }
    }
}

// === Provider test types and handler ===

#[derive(Deserialize)]
pub(crate) struct TestProviderRequest {
    pub provider: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct TestProviderResponse {
    pub ok: bool,
    pub message: String,
    pub models: Vec<String>,
}

pub(crate) async fn test_provider(
    Json(request): Json<TestProviderRequest>,
) -> Json<TestProviderResponse> {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Json(TestProviderResponse {
                ok: false,
                message: format!("Failed to create HTTP client: {e}"),
                models: Vec::new(),
            });
        }
    };

    let provider = request.provider.to_lowercase();

    match provider.as_str() {
        "ollama" => {
            let base_url = request
                .base_url
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let mut models = Vec::new();
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        if let Some(model_list) = body.get("models").and_then(|m| m.as_array()) {
                            for m in model_list {
                                if let Some(name) = m.get("name").and_then(|n| n.as_str()) {
                                    models.push(name.to_string());
                                }
                            }
                        }
                    }
                    Json(TestProviderResponse {
                        ok: true,
                        message: format!("Connected to Ollama. Found {} models.", models.len()),
                        models,
                    })
                }
                Ok(resp) => Json(TestProviderResponse {
                    ok: false,
                    message: format!("Ollama returned status {}", resp.status()),
                    models: Vec::new(),
                }),
                Err(e) => Json(TestProviderResponse {
                    ok: false,
                    message: format!("Failed to connect to Ollama at {url}: {e}"),
                    models: Vec::new(),
                }),
            }
        }
        "openai" | "openrouter" => {
            let default_base = if provider == "openrouter" {
                "https://openrouter.ai/api"
            } else {
                "https://api.openai.com"
            };
            let base_url = request.base_url.unwrap_or_else(|| default_base.to_string());
            let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
            let api_key = request.api_key.unwrap_or_default();
            if api_key.is_empty() {
                return Json(TestProviderResponse {
                    ok: false,
                    message: "API key is required".to_string(),
                    models: Vec::new(),
                });
            }
            let req = client
                .get(&url)
                .header("Authorization", format!("Bearer {api_key}"));
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    let mut models = Vec::new();
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        if let Some(data) = body.get("data").and_then(|d| d.as_array()) {
                            for m in data {
                                if let Some(id) = m.get("id").and_then(|i| i.as_str()) {
                                    models.push(id.to_string());
                                }
                            }
                        }
                    }
                    Json(TestProviderResponse {
                        ok: true,
                        message: format!(
                            "Connected to {}. Found {} models.",
                            provider,
                            models.len()
                        ),
                        models,
                    })
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    let msg = if status.as_u16() == 401 {
                        "Authentication failed. Check your API key.".to_string()
                    } else {
                        format!("{provider} returned status {status}: {body}")
                    };
                    Json(TestProviderResponse {
                        ok: false,
                        message: msg,
                        models: Vec::new(),
                    })
                }
                Err(e) => Json(TestProviderResponse {
                    ok: false,
                    message: format!("Failed to connect to {provider}: {e}"),
                    models: Vec::new(),
                }),
            }
        }
        "anthropic" => {
            let base_url = request
                .base_url
                .unwrap_or_else(|| "https://api.anthropic.com".to_string());
            let api_key = request.api_key.unwrap_or_default();
            if api_key.is_empty() {
                return Json(TestProviderResponse {
                    ok: false,
                    message: "API key is required".to_string(),
                    models: Vec::new(),
                });
            }
            let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "model": "claude-haiku-4-5-20251001",
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "hi"}]
            });
            let req = client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body);
            match req.send().await {
                Ok(resp) if resp.status().is_success() => Json(TestProviderResponse {
                    ok: true,
                    message: "Connected to Anthropic API.".to_string(),
                    models: vec![
                        "anthropic/claude-sonnet-4.5".to_string(),
                        "anthropic/claude-opus-4.5".to_string(),
                        "claude-haiku-4-5-20251001".to_string(),
                    ],
                }),
                Ok(resp) => {
                    let status = resp.status();
                    let body_text = resp.text().await.unwrap_or_default();
                    let msg = if status.as_u16() == 401 {
                        "Authentication failed. Check your API key.".to_string()
                    } else {
                        format!("Anthropic returned status {status}: {body_text}")
                    };
                    Json(TestProviderResponse {
                        ok: false,
                        message: msg,
                        models: Vec::new(),
                    })
                }
                Err(e) => Json(TestProviderResponse {
                    ok: false,
                    message: format!("Failed to connect to Anthropic: {e}"),
                    models: Vec::new(),
                }),
            }
        }
        _ => Json(TestProviderResponse {
            ok: false,
            message: format!("Unknown provider: {}", request.provider),
            models: Vec::new(),
        }),
    }
}
