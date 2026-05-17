//! GitHub App integration: OAuth device flow, webhooks, and Check Runs.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;
use tracing::{info, warn};

use super::state::{
    build_progress_callback, count_diff_files, count_reviewed_files, current_timestamp,
    emit_wide_event, AppState, FileMetricEvent, HotspotDetail, ReviewEventBuilder, ReviewSession,
    ReviewStatus,
};

// ── OAuth Device Flow ──────────────────────────────────────────────────

#[derive(Serialize)]
pub struct DeviceFlowResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// POST /api/gh/auth/device — start an OAuth device flow.
#[tracing::instrument(name = "github.device_flow_start", skip(state))]
pub async fn start_device_flow(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DeviceFlowResponse>, (StatusCode, String)> {
    let config = state.config.read().await;
    let client_id = config
        .github
        .client_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "GitHub App not configured. Set github_client_id in config.".to_string(),
            )
        })?
        .to_string();
    drop(config);

    let resp = state
        .http_client
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .form(&[("client_id", client_id.as_str()), ("scope", "repo")])
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("GitHub request failed: {e}"),
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("GitHub returned {status}: {body}"),
        ));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Failed to parse response: {e}"),
        )
    })?;

    Ok(Json(DeviceFlowResponse {
        device_code: body["device_code"].as_str().unwrap_or("").to_string(),
        user_code: body["user_code"].as_str().unwrap_or("").to_string(),
        verification_uri: body["verification_uri"]
            .as_str()
            .unwrap_or("https://github.com/login/device")
            .to_string(),
        expires_in: body["expires_in"].as_u64().unwrap_or(900),
        interval: body["interval"].as_u64().unwrap_or(5),
    }))
}

#[derive(Deserialize)]
pub struct PollDeviceFlowRequest {
    pub device_code: String,
}

#[derive(Serialize)]
pub struct PollDeviceFlowResponse {
    pub authenticated: bool,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub error: Option<String>,
}

/// POST /api/gh/auth/poll — poll for device flow completion.
#[tracing::instrument(name = "github.device_flow_poll", skip(state, request))]
pub async fn poll_device_flow(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PollDeviceFlowRequest>,
) -> Result<Json<PollDeviceFlowResponse>, (StatusCode, String)> {
    let config = state.config.read().await;
    let client_id = config
        .github
        .client_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "GitHub App not configured.".to_string(),
            )
        })?
        .to_string();
    drop(config);

    let resp = state
        .http_client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id.as_str()),
            ("device_code", request.device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("GitHub request failed: {e}"),
            )
        })?;

    let body: serde_json::Value = resp.json().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Failed to parse response: {e}"),
        )
    })?;

    // Check for errors (authorization_pending, slow_down, expired_token, etc.)
    if let Some(error) = body.get("error").and_then(|v| v.as_str()) {
        return Ok(Json(PollDeviceFlowResponse {
            authenticated: false,
            username: None,
            avatar_url: None,
            error: Some(error.to_string()),
        }));
    }

    // Got an access token
    let access_token = body["access_token"]
        .as_str()
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                "No access_token in response".to_string(),
            )
        })?
        .to_string();

    // Fetch user info with the new token
    let user_resp = state
        .http_client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "DiffScope")
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Failed to fetch user: {e}"),
            )
        })?;

    let user: serde_json::Value = user_resp.json().await.unwrap_or_default();

    let username = user["login"].as_str().map(|s| s.to_string());
    let avatar_url = user["avatar_url"].as_str().map(|s| s.to_string());

    // Store the token in config
    {
        let mut config = state.config.write().await;
        config.github.token = Some(access_token);
    }
    AppState::save_config_async(&state);

    info!(username = ?username, "GitHub OAuth device flow completed");

    Ok(Json(PollDeviceFlowResponse {
        authenticated: true,
        username,
        avatar_url,
        error: None,
    }))
}

/// DELETE /api/gh/auth — disconnect GitHub (clear token).
pub async fn disconnect_github(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    {
        let mut config = state.config.write().await;
        config.github.token = None;
    }
    AppState::save_config_async(&state);
    info!("GitHub disconnected");
    Json(serde_json::json!({ "ok": true }))
}

// ── Webhooks ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct WebhookStatusResponse {
    pub configured: bool,
    pub url: String,
}

/// GET /api/gh/webhook/status — return webhook configuration status.
pub async fn get_webhook_status(State(state): State<Arc<AppState>>) -> Json<WebhookStatusResponse> {
    let config = state.config.read().await;
    let configured = config
        .github
        .webhook_secret
        .as_ref()
        .is_some_and(|s| !s.is_empty());
    Json(WebhookStatusResponse {
        configured,
        url: "/api/webhooks/github".to_string(),
    })
}

/// POST /api/webhooks/github — receive GitHub webhook events.
#[tracing::instrument(name = "github.webhook", skip(state, headers, body), fields(event_type = tracing::field::Empty))]
pub async fn handle_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let config = state.config.read().await;

    // Verify signature if webhook secret is configured
    if let Some(ref secret) = config.github.webhook_secret {
        if !secret.is_empty() {
            let signature = headers
                .get("x-hub-signature-256")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    (
                        StatusCode::UNAUTHORIZED,
                        "Missing webhook signature".to_string(),
                    )
                })?;

            verify_webhook_signature(secret, &body, signature)
                .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
        }
    }

    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    tracing::Span::current().record("event_type", event_type);

    let token = config.github.token.clone();
    let github_app_id = config.github.app_id;
    let private_key = config.github.private_key.clone();
    let auto_review_events = config.github.auto_review_events.clone();
    let review_request_reviewers = config.github.review_request_reviewers.clone();
    drop(config);

    info!(event = %event_type, "Received GitHub webhook");

    match event_type {
        "pull_request" => {
            let payload: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid JSON: {e}")))?;

            let action = payload["action"].as_str().unwrap_or("");

            // Auto-review on configured pull_request actions. review_requested
            // is gated by requested reviewer login so an org webhook can stay
            // centralized without reviewing every PR.
            if should_start_review_for_pull_request_action(
                action,
                &payload,
                &auto_review_events,
                &review_request_reviewers,
            ) {
                let repo = payload["repository"]["full_name"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let pr_number = payload["pull_request"]["number"].as_u64().unwrap_or(0) as u32;
                let head_sha = payload["pull_request"]["head"]["sha"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let pr_title = payload["pull_request"]["title"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();

                if repo.is_empty() || pr_number == 0 {
                    return Err((StatusCode::BAD_REQUEST, "Invalid PR payload".to_string()));
                }

                info!(repo = %repo, pr = pr_number, action = %action, "Auto-reviewing PR");

                // Determine token to use: installation token (if app) or user token
                let auth_token =
                    if let (Some(app_id), Some(ref pkey)) = (github_app_id, &private_key) {
                        // Get installation token for this repo
                        let installation_id =
                            payload["installation"]["id"].as_u64().ok_or_else(|| {
                                (
                                    StatusCode::BAD_REQUEST,
                                    "No installation ID in webhook payload".to_string(),
                                )
                            })?;
                        get_installation_token(&state.http_client, app_id, pkey, installation_id)
                            .await
                            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
                    } else {
                        token.ok_or_else(|| {
                            (
                                StatusCode::BAD_REQUEST,
                                "No GitHub token configured".to_string(),
                            )
                        })?
                    };

                // Determine whether to fetch an incremental diff or the full PR diff.
                // For "synchronize" events, if we have a previously reviewed SHA,
                // fetch only the commits since that SHA via the compare API.
                let pr_key = format!("{repo}#{pr_number}");
                let last_reviewed_sha = if action == "synchronize" {
                    AppState::get_last_reviewed_sha(&state, &pr_key).await
                } else {
                    None
                };

                let (diff_content, is_incremental) = if let Some(ref base_sha) = last_reviewed_sha {
                    // Incremental: fetch only new commits since last review
                    info!(
                        repo = %repo,
                        pr = pr_number,
                        base_sha = %base_sha,
                        head_sha = %head_sha,
                        "Fetching incremental diff (push-by-push)"
                    );
                    let compare_url = format!(
                        "https://api.github.com/repos/{repo}/compare/{base_sha}...{head_sha}",
                    );
                    let compare_resp = state
                        .http_client
                        .get(&compare_url)
                        .header("Authorization", format!("Bearer {auth_token}"))
                        .header("Accept", "application/vnd.github.v3.diff")
                        .header("User-Agent", "DiffScope")
                        .send()
                        .await
                        .map_err(|e| {
                            (
                                StatusCode::BAD_GATEWAY,
                                format!("Failed to fetch incremental diff: {e}"),
                            )
                        })?;

                    if compare_resp.status().is_success() {
                        let content = compare_resp.text().await.unwrap_or_default();
                        if content.trim().is_empty() {
                            // No changes between SHAs (e.g. force push to same content);
                            // fall through to full diff below
                            info!(
                                repo = %repo,
                                pr = pr_number,
                                "Incremental diff empty, falling back to full PR diff"
                            );
                            (
                                fetch_full_pr_diff(&state, &auth_token, &repo, pr_number).await?,
                                false,
                            )
                        } else {
                            (content, true)
                        }
                    } else {
                        // Compare API failed (e.g. force push rewrote history);
                        // fall back to full PR diff
                        let status = compare_resp.status();
                        let body = compare_resp.text().await.unwrap_or_default();
                        warn!(
                            repo = %repo,
                            pr = pr_number,
                            status = %status,
                            "Incremental compare failed ({}), falling back to full PR diff: {}",
                            status,
                            body,
                        );
                        (
                            fetch_full_pr_diff(&state, &auth_token, &repo, pr_number).await?,
                            false,
                        )
                    }
                } else {
                    // No previous review or "opened" action — full PR diff
                    (
                        fetch_full_pr_diff(&state, &auth_token, &repo, pr_number).await?,
                        false,
                    )
                };

                let review_id = uuid::Uuid::new_v4().to_string();
                let diff_source = format!("pr:{repo}#{pr_number}");

                let session = ReviewSession {
                    id: review_id.clone(),
                    status: ReviewStatus::Pending,
                    diff_source: diff_source.clone(),
                    github_head_sha: Some(head_sha.clone()),
                    github_post_results_requested: None,
                    started_at: current_timestamp(),
                    completed_at: None,
                    comments: Vec::new(),
                    summary: None,
                    files_reviewed: 0,
                    error: None,
                    pr_summary_text: None,
                    diff_content: Some(diff_content.clone()),
                    event: None,
                    progress: None,
                };

                state
                    .reviews
                    .write()
                    .await
                    .insert(review_id.clone(), session);

                // Spawn review task with check run creation
                let state_clone = state.clone();
                let review_id_clone = review_id.clone();
                tokio::spawn(async move {
                    run_webhook_review(
                        state_clone,
                        WebhookReviewParams {
                            review_id: review_id_clone,
                            diff_content,
                            repo,
                            pr_number,
                            head_sha,
                            pr_title,
                            auth_token,
                            is_incremental,
                        },
                    )
                    .await;
                });

                return Ok(Json(serde_json::json!({
                    "ok": true,
                    "action": "review_started",
                    "review_id": review_id,
                    "incremental": is_incremental,
                })));
            }
        }
        "issue_comment" => {
            let payload: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid JSON: {e}")))?;

            let action = payload["action"].as_str().unwrap_or("");
            let comment_body = payload["comment"]["body"].as_str().unwrap_or("");

            // Only process new comments that contain @diffscope commands
            if action == "created" {
                if let Some(cmd) = crate::core::interactive::InteractiveCommand::parse(comment_body)
                {
                    let repo = payload["repository"]["full_name"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let issue_number = payload["issue"]["number"].as_u64().unwrap_or(0) as u32;

                    info!(
                        repo = %repo,
                        issue = issue_number,
                        command = ?cmd.command,
                        "Processing @diffscope command"
                    );

                    // Execute the command
                    let response_body = match cmd.command {
                        crate::core::interactive::CommandType::Help => {
                            crate::core::interactive::InteractiveCommand::help_text()
                        }
                        _ => {
                            // Create adapter from server config for LLM commands
                            let config = state.config.read().await;
                            let model_config = config.to_model_config();
                            drop(config);

                            match crate::adapters::llm::create_adapter(&model_config) {
                                Ok(adapter) => match cmd.execute(adapter.as_ref(), None).await {
                                    Ok(result) => result,
                                    Err(e) => format!("Command failed: {e}"),
                                },
                                Err(e) => format!("Failed to create adapter: {e}"),
                            }
                        }
                    };

                    // Post response comment if we have a token
                    if let Some(ref auth) = token {
                        let comment_url = format!(
                            "https://api.github.com/repos/{repo}/issues/{issue_number}/comments"
                        );
                        let _ = state
                            .http_client
                            .post(&comment_url)
                            .header("Authorization", format!("Bearer {auth}"))
                            .header("User-Agent", "DiffScope")
                            .json(&serde_json::json!({ "body": response_body }))
                            .send()
                            .await;
                    }

                    return Ok(Json(serde_json::json!({
                        "ok": true,
                        "action": "command_processed",
                        "command": format!("{:?}", cmd.command),
                    })));
                }
            }
        }
        "ping" => {
            info!("GitHub webhook ping received");
            return Ok(Json(serde_json::json!({ "ok": true, "action": "pong" })));
        }
        _ => {}
    }

    Ok(Json(serde_json::json!({ "ok": true, "action": "ignored" })))
}

fn should_start_review_for_pull_request_action(
    action: &str,
    payload: &serde_json::Value,
    auto_review_events: &[String],
    review_request_reviewers: &[String],
) -> bool {
    if !matches!(
        action,
        "opened" | "synchronize" | "reopened" | "review_requested"
    ) {
        return false;
    }
    if !auto_review_events
        .iter()
        .any(|event| event.eq_ignore_ascii_case(action))
    {
        return false;
    }
    if action != "review_requested" {
        return true;
    }

    let Some(login) = requested_reviewer_login(payload) else {
        return false;
    };
    review_request_reviewers
        .iter()
        .any(|reviewer| reviewer.trim().eq_ignore_ascii_case(login.trim()))
}

fn requested_reviewer_login(payload: &serde_json::Value) -> Option<&str> {
    payload
        .get("requested_reviewer")
        .and_then(|reviewer| reviewer.get("login"))
        .and_then(|login| login.as_str())
}

fn verify_webhook_signature(secret: &str, body: &str, signature: &str) -> Result<(), String> {
    let sig_hex = signature
        .strip_prefix("sha256=")
        .ok_or_else(|| "Invalid signature format".to_string())?;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("HMAC init failed: {e}"))?;
    mac.update(body.as_bytes());

    let expected = hex::encode(mac.finalize().into_bytes());

    // Constant-time comparison
    if expected.len() != sig_hex.len() {
        return Err("Signature mismatch".to_string());
    }
    let matches = expected
        .bytes()
        .zip(sig_hex.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    if matches != 0 {
        return Err("Signature mismatch".to_string());
    }
    Ok(())
}

/// Hex-encode bytes (avoids adding hex crate).
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
}

// ── GitHub App Installation Tokens ─────────────────────────────────────

/// Create a JWT for GitHub App authentication.
fn create_app_jwt(app_id: u64, private_key_pem: &str) -> Result<String, String> {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    #[derive(serde::Serialize)]
    struct Claims {
        iat: u64,
        exp: u64,
        iss: String,
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Time error: {e}"))?
        .as_secs();

    let claims = Claims {
        iat: now.saturating_sub(60), // 60s clock skew tolerance
        exp: now + 600,              // 10 minute expiry (max allowed)
        iss: app_id.to_string(),
    };

    let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
        .map_err(|e| format!("Invalid private key: {e}"))?;

    encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|e| format!("JWT encoding failed: {e}"))
}

/// Get an installation access token for a specific installation.
async fn get_installation_token(
    client: &reqwest::Client,
    app_id: u64,
    private_key_pem: &str,
    installation_id: u64,
) -> Result<String, String> {
    let jwt = create_app_jwt(app_id, private_key_pem)?;

    let url = format!("https://api.github.com/app/installations/{installation_id}/access_tokens",);

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {jwt}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "DiffScope")
        .send()
        .await
        .map_err(|e| format!("Installation token request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub returned {status}: {body}"));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))?;

    body["token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No token in installation response".to_string())
}

// ── Check Runs ─────────────────────────────────────────────────────────

/// Create a check run on a commit with DiffScope review results.
async fn create_check_run(
    client: &reqwest::Client,
    token: &str,
    repo: &str,
    head_sha: &str,
    title: &str,
    comments: &[crate::core::Comment],
    summary: &crate::core::comment::ReviewSummary,
) -> Result<(), String> {
    // Build annotations from comments (max 50 per API call)
    let annotations: Vec<serde_json::Value> = comments
        .iter()
        .take(50)
        .map(|c| {
            let path = c.file_path.display().to_string();
            let path = path.trim_start_matches('/');
            let path = if path.starts_with("a/") || path.starts_with("b/") {
                &path[2..]
            } else {
                path
            };

            let level = match c.severity {
                crate::core::comment::Severity::Error => "failure",
                crate::core::comment::Severity::Warning => "warning",
                _ => "notice",
            };

            serde_json::json!({
                "path": path,
                "start_line": c.line_number,
                "end_line": c.line_number,
                "annotation_level": level,
                "title": format!("{}: {}", c.severity, c.category),
                "message": c.content,
            })
        })
        .collect();

    // Determine conclusion
    let has_errors = comments
        .iter()
        .any(|c| matches!(c.severity, crate::core::comment::Severity::Error));
    let conclusion = match summary.merge_readiness {
        crate::core::comment::MergeReadiness::NeedsReReview => "neutral",
        _ if has_errors || summary.open_blockers > 0 => "failure",
        _ => "success",
    };

    let summary_text = format!(
        "**Score:** {:.1}/10 | **Findings:** {} | **Files:** {} | **Readiness:** {}\n**Completeness:** {} acknowledged | {} fixed | {} stale\n\n{}{}",
        summary.overall_score,
        summary.total_comments,
        summary.files_reviewed,
        summary.merge_readiness,
        summary.completeness.acknowledged_findings,
        summary.completeness.fixed_findings,
        summary.completeness.stale_findings,
        if summary.recommendations.is_empty() {
            String::new()
        } else {
            format!(
                "**Recommendations:**\n{}",
                summary
                    .recommendations
                    .iter()
                    .map(|r| format!("- {r}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        },
        if summary.readiness_reasons.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n**Review State:**\n{}",
                summary
                    .readiness_reasons
                    .iter()
                    .map(|reason| format!("- {}", reason))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        },
    );

    let check_run = serde_json::json!({
        "name": "DiffScope Review",
        "head_sha": head_sha,
        "status": "completed",
        "conclusion": conclusion,
        "output": {
            "title": title,
            "summary": summary_text,
            "annotations": annotations,
        },
    });

    let url = format!("https://api.github.com/repos/{repo}/check-runs");

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "DiffScope")
        .json(&check_run)
        .send()
        .await
        .map_err(|e| format!("Check run request failed: {e}"))?;

    if resp.status().is_success() {
        info!(repo = %repo, sha = %head_sha, conclusion = %conclusion, "Created check run");
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        warn!(repo = %repo, status = %status, "Failed to create check run: {}", body);
        Err(format!("GitHub returned {status}: {body}"))
    }
}

// ── Post PR Summary Comment ─────────────────────────────────────────────

/// Post an AI-generated PR summary as a standalone issue comment on the PR.
async fn post_pr_summary_comment(
    client: &reqwest::Client,
    token: &str,
    repo: &str,
    pr_number: u32,
    summary_markdown: &str,
) -> Result<(), String> {
    let url = format!("https://api.github.com/repos/{repo}/issues/{pr_number}/comments",);

    let body = serde_json::json!({
        "body": summary_markdown,
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "DiffScope")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to post PR summary comment: {e}"))?;

    if resp.status().is_success() {
        info!(repo = %repo, pr = pr_number, "Posted PR summary comment");
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("GitHub returned {status}: {body}"))
    }
}

// ── Webhook-triggered review task ──────────────────────────────────────

struct WebhookReviewParams {
    review_id: String,
    diff_content: String,
    repo: String,
    pr_number: u32,
    head_sha: String,
    pr_title: String,
    auth_token: String,
    /// Whether this review covers only incremental changes (push-by-push delta).
    is_incremental: bool,
}

/// Fetch the full PR diff from the GitHub API.
async fn fetch_full_pr_diff(
    state: &Arc<AppState>,
    auth_token: &str,
    repo: &str,
    pr_number: u32,
) -> Result<String, (StatusCode, String)> {
    let diff_url = format!("https://api.github.com/repos/{repo}/pulls/{pr_number}");
    let diff_resp = state
        .http_client
        .get(&diff_url)
        .header("Authorization", format!("Bearer {auth_token}"))
        .header("Accept", "application/vnd.github.v3.diff")
        .header("User-Agent", "DiffScope")
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Failed to fetch diff: {e}"),
            )
        })?;

    if !diff_resp.status().is_success() {
        let status = diff_resp.status();
        let body = diff_resp.text().await.unwrap_or_default();
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("GitHub returned {status}: {body}"),
        ));
    }

    diff_resp.text().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Failed to read diff body: {e}"),
        )
    })
}

#[tracing::instrument(name = "github.webhook_review", skip(state, params), fields(review_id = %params.review_id, repo = %params.repo, pr_number = params.pr_number, diff_bytes = params.diff_content.len(), incremental = params.is_incremental))]
async fn run_webhook_review(state: Arc<AppState>, params: WebhookReviewParams) {
    let WebhookReviewParams {
        review_id,
        diff_content,
        repo,
        pr_number,
        head_sha,
        pr_title,
        auth_token,
        is_incremental,
    } = params;
    use crate::core::comment::CommentSynthesizer;

    let _permit = match state.review_semaphore.clone().acquire_owned().await {
        Ok(permit) => permit,
        Err(_) => {
            AppState::fail_review(
                &state,
                &review_id,
                "Review semaphore closed".to_string(),
                None,
            )
            .await;
            return;
        }
    };

    let task_start = std::time::Instant::now();
    let diff_source = format!("pr:{repo}#{pr_number}");
    let pr_key = format!("{repo}#{pr_number}");

    AppState::mark_running(&state, &review_id).await;

    let config = state.config.read().await.clone();
    let repo_path = state.repo_path.clone();
    let model = config.generation_model_name().to_string();
    let generation_role = config.generation_model_role.as_str().to_string();
    let provider = config.inferred_provider_label_for_role(config.generation_model_role);
    let base_url = config.base_url.clone();
    let summary_config = if config.smart_review_summary {
        Some(config.clone())
    } else {
        None
    };

    let diff_bytes = diff_content.len();
    let diff_files_total = count_diff_files(&diff_content);

    if diff_content.trim().is_empty() {
        let event = ReviewEventBuilder::new(&review_id, "review.completed", &diff_source, &model)
            .provider(provider.as_deref())
            .base_url(base_url.as_deref())
            .duration_ms(task_start.elapsed().as_millis() as u64)
            .github(&repo, pr_number)
            .build();
        emit_wide_event(&event);
        AppState::complete_review(
            &state,
            &review_id,
            Vec::new(),
            CommentSynthesizer::generate_summary(&[]),
            0,
            event,
        )
        .await;
        super::api::persist_pr_fix_loop_telemetry(&state, &review_id, &repo, pr_number).await;
        // Record the reviewed SHA for future incremental reviews
        AppState::record_reviewed_sha(&state, &pr_key, &head_sha).await;
        AppState::save_reviews_async(&state);
        return;
    }

    let on_progress: Option<crate::review::ProgressCallback> =
        Some(build_progress_callback(&state, &review_id, task_start));

    let llm_start = std::time::Instant::now();
    let verification_reuse_cache = AppState::get_pr_verification_reuse_cache(&state, &pr_key).await;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        crate::review::review_diff_content_raw_with_progress_and_verification_reuse(
            &diff_content,
            config,
            &repo_path,
            on_progress,
            verification_reuse_cache,
        ),
    )
    .await;
    let llm_ms = llm_start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(review_result)) => {
            let verification_reuse_cache = review_result.verification_reuse_cache.clone();
            let comments = review_result.comments;
            let summary = CommentSynthesizer::apply_verification(
                CommentSynthesizer::generate_summary(&comments),
                crate::review::summarize_review_verification(
                    review_result.verification_report.as_ref(),
                    &review_result.warnings,
                ),
            );
            let files_reviewed = count_reviewed_files(&comments);

            // Post inline review comments to PR
            let mut github_posted = false;
            if !comments.is_empty() {
                github_posted = super::api::post_pr_review_comments(
                    &state.http_client,
                    &auth_token,
                    &repo,
                    pr_number,
                    &comments,
                    Some(&summary),
                )
                .await
                .is_ok();
            }

            // Create Check Run
            let incremental_tag = if is_incremental { " (incremental)" } else { "" };
            let check_title = if pr_title.is_empty() {
                format!(
                    "DiffScope{}: {}/{}",
                    incremental_tag, summary.total_comments, summary.overall_score
                )
            } else {
                format!(
                    "DiffScope{}: {} — {:.1}/10",
                    incremental_tag, pr_title, summary.overall_score
                )
            };
            let _ = create_check_run(
                &state.http_client,
                &auth_token,
                &repo,
                &head_sha,
                &check_title,
                &comments,
                &summary,
            )
            .await;

            let file_metric_events: Vec<FileMetricEvent> = review_result
                .file_metrics
                .iter()
                .map(|m| FileMetricEvent {
                    file_path: m.file_path.display().to_string(),
                    latency_ms: m.latency_ms,
                    prompt_tokens: m.prompt_tokens,
                    completion_tokens: m.completion_tokens,
                    total_tokens: m.total_tokens,
                    comment_count: m.comment_count,
                })
                .collect();
            let event =
                ReviewEventBuilder::new(&review_id, "review.completed", &diff_source, &model)
                    .provider(provider.as_deref())
                    .base_url(base_url.as_deref())
                    .duration_ms(task_start.elapsed().as_millis() as u64)
                    .llm_total_ms(llm_ms)
                    .diff_stats(
                        diff_bytes,
                        diff_files_total,
                        files_reviewed,
                        diff_files_total.saturating_sub(files_reviewed),
                    )
                    .comments(&comments, Some(&summary))
                    .tokens(
                        review_result.total_prompt_tokens,
                        review_result.total_completion_tokens,
                        review_result.total_tokens,
                    )
                    .file_metrics(file_metric_events)
                    .hotspot_details(
                        review_result
                            .hotspots
                            .iter()
                            .map(|h| HotspotDetail {
                                file_path: h.file_path.display().to_string(),
                                risk_score: h.risk_score,
                                reasons: h.reasons.clone(),
                            })
                            .collect(),
                    )
                    .convention_suppressed(review_result.convention_suppressed_count)
                    .comments_by_pass(review_result.comments_by_pass)
                    .cost_breakdowns(crate::server::cost::review_cost_breakdowns(
                        crate::server::cost::CostBreakdownRequest {
                            workload: "review_generation",
                            role: &generation_role,
                            provider: provider.clone(),
                            model: &model,
                            prompt_tokens: review_result.total_prompt_tokens,
                            completion_tokens: review_result.total_completion_tokens,
                            total_tokens: review_result.total_tokens,
                        },
                        "review_verification",
                        review_result.verification_report.as_ref(),
                    ))
                    .github(&repo, pr_number)
                    .github_posted(github_posted)
                    .build();
            emit_wide_event(&event);
            super::api::record_pr_follow_up_outcome_feedback(
                &state,
                &repo,
                pr_number,
                &head_sha,
                &comments,
                &auth_token,
            )
            .await;
            AppState::complete_review(&state, &review_id, comments, summary, files_reviewed, event)
                .await;
            AppState::store_pr_verification_reuse_cache(&state, &pr_key, verification_reuse_cache)
                .await;
            super::api::persist_pr_fix_loop_telemetry(&state, &review_id, &repo, pr_number).await;

            // Record the reviewed SHA for future incremental reviews
            AppState::record_reviewed_sha(&state, &pr_key, &head_sha).await;
            if is_incremental {
                info!(
                    repo = %repo,
                    pr = pr_number,
                    head_sha = %head_sha,
                    "Incremental review completed, updated last reviewed SHA"
                );
            }

            // Generate AI-powered PR summary and post it as a comment if enabled
            if let Some(ref cfg) = summary_config {
                super::api::generate_and_store_pr_summary(&state, &review_id, &diff_content, cfg)
                    .await;

                // Post the summary as a PR comment
                let pr_summary_text = {
                    let reviews = state.reviews.read().await;
                    reviews
                        .get(&review_id)
                        .and_then(|s| s.pr_summary_text.clone())
                };
                if let Some(summary_md) = pr_summary_text {
                    if let Err(e) = post_pr_summary_comment(
                        &state.http_client,
                        &auth_token,
                        &repo,
                        pr_number,
                        &summary_md,
                    )
                    .await
                    {
                        warn!(review_id = %review_id, "Failed to post PR summary comment: {}", e);
                    }
                }
            }
        }
        Ok(Err(e)) => {
            let err_msg = format!("Review failed: {e}");
            warn!(review_id = %review_id, error = %err_msg, "Webhook review failed");
            let event = ReviewEventBuilder::new(&review_id, "review.failed", &diff_source, &model)
                .provider(provider.as_deref())
                .base_url(base_url.as_deref())
                .duration_ms(task_start.elapsed().as_millis() as u64)
                .llm_total_ms(llm_ms)
                .diff_stats(diff_bytes, diff_files_total, 0, 0)
                .github(&repo, pr_number)
                .error(&err_msg)
                .build();
            emit_wide_event(&event);
            AppState::fail_review(&state, &review_id, err_msg, Some(event)).await;
        }
        Err(_) => {
            let err_msg = "Review timed out after 5 minutes".to_string();
            warn!(review_id = %review_id, "Webhook review timed out");
            let event = ReviewEventBuilder::new(&review_id, "review.timeout", &diff_source, &model)
                .provider(provider.as_deref())
                .base_url(base_url.as_deref())
                .duration_ms(task_start.elapsed().as_millis() as u64)
                .llm_total_ms(llm_ms)
                .diff_stats(diff_bytes, diff_files_total, 0, 0)
                .github(&repo, pr_number)
                .error(&err_msg)
                .build();
            emit_wide_event(&event);
            AppState::fail_review(&state, &review_id, err_msg, Some(event)).await;
        }
    }

    AppState::save_reviews_async(&state);
    AppState::prune_old_reviews(&state).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex::encode([]), "");
        assert_eq!(hex::encode([0x00]), "00");
        assert_eq!(hex::encode([0xff]), "ff");
        assert_eq!(hex::encode([0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(
            hex::encode([0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]),
            "0123456789abcdef"
        );
    }

    #[test]
    fn test_verify_webhook_signature_valid() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let secret = "test-webhook-secret";
        let body = r#"{"action":"opened","pull_request":{"number":1}}"#;

        // Compute the expected signature
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());
        let signature = format!("sha256={expected}");

        assert!(verify_webhook_signature(secret, body, &signature).is_ok());
    }

    #[test]
    fn review_request_action_matches_configured_reviewer() {
        let payload = json!({
            "action": "review_requested",
            "requested_reviewer": { "login": "evalopsbot" }
        });
        let events = vec!["review_requested".to_string()];
        let reviewers = vec!["EvalOpsBot".to_string()];

        assert!(should_start_review_for_pull_request_action(
            "review_requested",
            &payload,
            &events,
            &reviewers
        ));
    }

    #[test]
    fn review_request_action_ignores_unconfigured_reviewer() {
        let payload = json!({
            "action": "review_requested",
            "requested_reviewer": { "login": "other-bot" }
        });
        let events = vec!["review_requested".to_string()];
        let reviewers = vec!["EvalOpsBot".to_string()];

        assert!(!should_start_review_for_pull_request_action(
            "review_requested",
            &payload,
            &events,
            &reviewers
        ));
    }

    #[test]
    fn review_request_action_requires_explicit_event_enablement() {
        let payload = json!({
            "action": "review_requested",
            "requested_reviewer": { "login": "EvalOpsBot" }
        });
        let events = vec!["opened".to_string(), "synchronize".to_string()];
        let reviewers = vec!["EvalOpsBot".to_string()];

        assert!(!should_start_review_for_pull_request_action(
            "review_requested",
            &payload,
            &events,
            &reviewers
        ));
    }

    #[test]
    fn opened_action_matches_when_enabled_without_reviewer_gate() {
        let payload = json!({});
        let events = vec!["opened".to_string()];
        let reviewers = Vec::new();

        assert!(should_start_review_for_pull_request_action(
            "opened", &payload, &events, &reviewers
        ));
    }

    #[test]
    fn test_verify_webhook_signature_invalid() {
        let secret = "test-secret";
        let body = "test body";
        let bad_sig = "sha256=0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_webhook_signature(secret, body, bad_sig).is_err());
    }

    #[test]
    fn test_verify_webhook_signature_missing_prefix() {
        let secret = "test-secret";
        let body = "test body";
        let no_prefix = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        assert!(verify_webhook_signature(secret, body, no_prefix).is_err());
    }

    #[test]
    fn test_verify_webhook_signature_wrong_length() {
        let secret = "test-secret";
        let body = "test body";
        let short_sig = "sha256=abcdef";
        assert!(verify_webhook_signature(secret, body, short_sig).is_err());
    }

    #[test]
    fn test_verify_webhook_signature_empty_body() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let secret = "my-secret";
        let body = "";

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());
        let signature = format!("sha256={expected}");

        assert!(verify_webhook_signature(secret, body, &signature).is_ok());
    }
}
