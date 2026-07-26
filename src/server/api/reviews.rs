use super::*;

pub(crate) async fn get_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let config = state.config.read().await;
    let reviews = state.reviews.read().await;

    let branch = git2::Repository::discover(&state.repo_path)
        .ok()
        .and_then(|repo| {
            repo.head()
                .ok()
                .and_then(|h| h.shorthand().ok().map(|s| s.to_string()))
        });

    Json(StatusResponse {
        repo_path: state.repo_path.display().to_string(),
        branch,
        model: config.generation_model_name().to_string(),
        adapter: config.adapter.clone(),
        base_url: config.base_url.clone(),
        active_reviews: reviews
            .values()
            .filter(|r| r.status == ReviewStatus::Running)
            .count(),
    })
}

pub(crate) async fn get_agent_tools() -> Json<Vec<crate::core::agent_tools::AgentToolInfo>> {
    Json(crate::core::agent_tools::list_all_tool_info())
}

#[tracing::instrument(name = "api.start_review", skip(state, request), fields(diff_source = %request.diff_source))]
pub(crate) async fn start_review(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartReviewRequest>,
) -> Result<Json<StartReviewResponse>, (StatusCode, String)> {
    // Validate diff_source
    let diff_source = match request.diff_source.as_str() {
        "head" | "staged" | "branch" | "raw" => request.diff_source.clone(),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid diff_source: must be head, staged, branch, or raw".to_string(),
            ))
        }
    };

    // "raw" requires diff_content
    if diff_source == "raw"
        && request
            .diff_content
            .as_ref()
            .is_none_or(|c| c.trim().is_empty())
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "diff_content is required when diff_source is 'raw'".to_string(),
        ));
    }

    // Reject oversized diffs
    if let Some(ref content) = request.diff_content {
        if content.len() > MAX_DIFF_SIZE {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "Diff content exceeds maximum size of {} MB",
                    MAX_DIFF_SIZE / (1024 * 1024)
                ),
            ));
        }
    }

    info!(diff_source = %diff_source, title = ?request.title, "Starting review");

    // Validate branch name if provided
    if let Some(ref branch) = request.base_branch {
        if branch.is_empty()
            || branch.len() > 200
            || !branch
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'))
        {
            return Err((StatusCode::BAD_REQUEST, "Invalid branch name".to_string()));
        }
    }

    let id = Uuid::new_v4().to_string();

    let display_source = if diff_source == "raw" {
        request.title.clone().unwrap_or_else(|| "raw".to_string())
    } else {
        diff_source.clone()
    };

    let session = ReviewSession {
        id: id.clone(),
        status: ReviewStatus::Pending,
        diff_source: display_source,
        github_head_sha: None,
        github_post_results_requested: None,
        started_at: current_timestamp(),
        completed_at: None,
        comments: Vec::new(),
        summary: None,
        files_reviewed: 0,
        error: None,
        pr_summary_text: None,
        diff_content: None,
        event: None,
        progress: None,
    };

    state.reviews.write().await.insert(id.clone(), session);

    let state_clone = state.clone();
    let review_id = id.clone();
    let base_branch = request.base_branch.clone();
    let raw_diff = request.diff_content.clone();
    let overrides = ReviewOverrides {
        model: request.model.clone(),
        strictness: request.strictness,
        review_profile: request.review_profile.clone(),
    };

    tokio::spawn(async move {
        run_review_task(
            state_clone,
            review_id,
            diff_source,
            base_branch,
            raw_diff,
            overrides,
        )
        .await;
    });

    Ok(Json(StartReviewResponse {
        id,
        status: ReviewStatus::Pending,
    }))
}

#[tracing::instrument(name = "review_task", skip(state, raw_diff, overrides), fields(review_id = %review_id, diff_source = %diff_source))]
pub(crate) async fn run_review_task(
    state: Arc<AppState>,
    review_id: String,
    diff_source: String,
    base_branch: Option<String>,
    raw_diff: Option<String>,
    overrides: ReviewOverrides,
) {
    // Acquire semaphore permit to limit concurrent reviews
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
    AppState::mark_running(&state, &review_id).await;

    let mut config = state.config.read().await.clone();
    let repo_path = state.repo_path.clone();

    // Apply per-review overrides
    if let Some(m) = overrides.model {
        config.set_model_for_role(config.generation_model_role, m);
    }
    if let Some(s) = overrides.strictness {
        config.strictness = s.clamp(1, 3);
    }
    if let Some(p) = overrides.review_profile {
        config.review_profile = Some(p);
    }

    let model = config.generation_model_name().to_string();
    let generation_role = config.generation_model_role.as_str().to_string();
    let provider = config.inferred_provider_label_for_role(config.generation_model_role);
    let base_url = config.base_url.clone();

    // Get the diff content based on source
    let diff_fetch_start = std::time::Instant::now();
    let diff_result = if diff_source == "raw" {
        Ok(raw_diff.unwrap_or_default())
    } else {
        match diff_source.as_str() {
            "staged" => get_diff_from_git(&repo_path, "staged", None),
            "branch" => {
                let base = base_branch.as_deref().unwrap_or("main");
                get_diff_from_git(&repo_path, "branch", Some(base))
            }
            _ => get_diff_from_git(&repo_path, "head", None),
        }
    };
    let diff_fetch_ms = diff_fetch_start.elapsed().as_millis() as u64;

    let diff_content = match diff_result {
        Ok(diff) => diff,
        Err(e) => {
            let err_msg = format!("Failed to get diff: {e}");
            let event = ReviewEventBuilder::new(&review_id, "review.failed", &diff_source, &model)
                .provider(provider.as_deref())
                .base_url(base_url.as_deref())
                .duration_ms(task_start.elapsed().as_millis() as u64)
                .diff_fetch_ms(diff_fetch_ms)
                .error(&err_msg)
                .build();
            emit_wide_event(&event);
            AppState::fail_review(&state, &review_id, err_msg, Some(event)).await;
            capture_review_forensics(
                &state,
                &config,
                &review_id,
                "review_failed",
                crate::forensics::ReviewForensicsRuntime {
                    diff_source: diff_source.clone(),
                    model: model.clone(),
                    provider: provider.clone(),
                    base_url: base_url.clone(),
                    warnings: vec!["Diff fetch failed before review execution.".to_string()],
                    ..Default::default()
                },
            )
            .await;
            AppState::save_reviews_async(&state);
            return;
        }
    };

    let diff_bytes = diff_content.len();
    let diff_files_total = count_diff_files(&diff_content);

    // Store diff content for the frontend viewer
    {
        let mut reviews = state.reviews.write().await;
        if let Some(session) = reviews.get_mut(&review_id) {
            session.diff_content = Some(diff_content.clone());
        }
    }

    if diff_content.trim().is_empty() {
        let event = ReviewEventBuilder::new(&review_id, "review.completed", &diff_source, &model)
            .provider(provider.as_deref())
            .base_url(base_url.as_deref())
            .duration_ms(task_start.elapsed().as_millis() as u64)
            .diff_fetch_ms(diff_fetch_ms)
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
        AppState::save_reviews_async(&state);
        return;
    }

    let on_progress = Some(build_progress_callback(&state, &review_id, task_start));

    // Save config for post-review PR summary generation (config is moved into the review call)
    let summary_config = if config.smart_review_summary {
        Some(config.clone())
    } else {
        None
    };

    // Run the review with a 5-minute timeout
    let llm_start = std::time::Instant::now();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        crate::review::review_diff_content_raw_with_progress(
            &diff_content,
            config.clone(),
            &repo_path,
            on_progress,
        ),
    )
    .await;
    let llm_ms = llm_start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(review_result)) => {
            let comments = review_result.comments;
            let summary = CommentSynthesizer::apply_verification(
                CommentSynthesizer::generate_summary(&comments),
                crate::review::summarize_review_verification(
                    review_result.verification_report.as_ref(),
                    &review_result.warnings,
                ),
            );
            let files_reviewed = count_reviewed_files(&comments);
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
                    .diff_fetch_ms(diff_fetch_ms)
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
                    .agent_activity(review_result.agent_activity.as_ref())
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
                    .build();
            emit_wide_event(&event);
            AppState::complete_review(&state, &review_id, comments, summary, files_reviewed, event)
                .await;

            // Generate AI-powered PR summary if enabled
            if let Some(ref cfg) = summary_config {
                generate_and_store_pr_summary(&state, &review_id, &diff_content, cfg).await;
            }

            if !review_result.warnings.is_empty() {
                capture_review_forensics(
                    &state,
                    &config,
                    &review_id,
                    "review_warnings",
                    crate::forensics::ReviewForensicsRuntime {
                        diff_source: diff_source.clone(),
                        model: model.clone(),
                        provider: provider.clone(),
                        base_url: base_url.clone(),
                        warnings: review_result.warnings,
                        verification_report: review_result.verification_report,
                        agent_activity: review_result
                            .agent_activity
                            .as_ref()
                            .map(crate::forensics::ReviewForensicsAgentActivity::from),
                        dag_traces: review_result.dag_traces,
                    },
                )
                .await;
            }
        }
        Ok(Err(e)) => {
            let err_msg = format!("Review failed: {e}");
            let event = ReviewEventBuilder::new(&review_id, "review.failed", &diff_source, &model)
                .provider(provider.as_deref())
                .base_url(base_url.as_deref())
                .duration_ms(task_start.elapsed().as_millis() as u64)
                .diff_fetch_ms(diff_fetch_ms)
                .llm_total_ms(llm_ms)
                .diff_stats(diff_bytes, diff_files_total, 0, 0)
                .error(&err_msg)
                .build();
            emit_wide_event(&event);
            AppState::fail_review(&state, &review_id, err_msg, Some(event)).await;
            capture_review_forensics(
                &state,
                &config,
                &review_id,
                "review_failed",
                crate::forensics::ReviewForensicsRuntime {
                    diff_source: diff_source.clone(),
                    model: model.clone(),
                    provider: provider.clone(),
                    base_url: base_url.clone(),
                    warnings: vec![e.to_string()],
                    ..Default::default()
                },
            )
            .await;
        }
        Err(_) => {
            let err_msg = "Review timed out after 5 minutes".to_string();
            let event = ReviewEventBuilder::new(&review_id, "review.timeout", &diff_source, &model)
                .provider(provider.as_deref())
                .base_url(base_url.as_deref())
                .duration_ms(task_start.elapsed().as_millis() as u64)
                .diff_fetch_ms(diff_fetch_ms)
                .llm_total_ms(llm_ms)
                .diff_stats(diff_bytes, diff_files_total, 0, 0)
                .error(&err_msg)
                .build();
            emit_wide_event(&event);
            AppState::fail_review(&state, &review_id, err_msg, Some(event)).await;
            capture_review_forensics(
                &state,
                &config,
                &review_id,
                "review_timeout",
                crate::forensics::ReviewForensicsRuntime {
                    diff_source: diff_source.clone(),
                    model: model.clone(),
                    provider: provider.clone(),
                    base_url: base_url.clone(),
                    warnings: vec!["Review timed out after 5 minutes.".to_string()],
                    ..Default::default()
                },
            )
            .await;
        }
    }

    AppState::save_reviews_async(&state);
    AppState::prune_old_reviews(&state).await;

    // Persist to storage backend (PostgreSQL or JSON)
    {
        let reviews = state.reviews.read().await;
        if let Some(session) = reviews.get(&review_id) {
            if let Err(e) = state.storage.save_review(session).await {
                warn!(review_id = %review_id, "Failed to persist review to storage: {}", e);
            }
            if let Some(ref event) = session.event {
                if let Err(e) = state.storage.save_event(event).await {
                    warn!(review_id = %review_id, "Failed to persist event to storage: {}", e);
                }
            }
        }
    }
}

async fn capture_review_forensics(
    state: &Arc<AppState>,
    config: &crate::config::Config,
    review_id: &str,
    trigger: &str,
    runtime: crate::forensics::ReviewForensicsRuntime,
) {
    let session = {
        let reviews = state.reviews.read().await;
        reviews.get(review_id).cloned()
    };
    let Some(session) = session else {
        return;
    };

    if let Err(error) = crate::forensics::write_review_forensics_bundle(
        config,
        crate::forensics::ReviewForensicsBundleInput {
            review_id: review_id.to_string(),
            trigger: trigger.to_string(),
            session,
            runtime,
        },
    )
    .await
    {
        warn!(review_id = %review_id, "Failed to write review forensics bundle: {error}");
    }
}

pub(crate) fn get_diff_from_git(
    repo_path: &std::path::Path,
    source: &str,
    base: Option<&str>,
) -> anyhow::Result<String> {
    use std::process::Command;

    let output = match source {
        "staged" => Command::new("git")
            .args(["diff", "--cached"])
            .current_dir(repo_path)
            .output()?,
        "branch" => {
            let base_branch = base.unwrap_or("main");
            Command::new("git")
                .args(["diff", &format!("{base_branch}...HEAD")])
                .current_dir(repo_path)
                .output()?
        }
        _ => Command::new("git")
            .args(["diff", "HEAD~1"])
            .current_dir(repo_path)
            .output()?,
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub(crate) async fn get_review(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiReviewSession>, StatusCode> {
    let session = if let Some(session) = {
        let reviews = state.reviews.read().await;
        reviews.get(&id).cloned()
    } {
        session
    } else {
        match state.storage.get_review(&id).await {
            Ok(Some(session)) => session,
            _ => return Err(StatusCode::NOT_FOUND),
        }
    };

    let inventory = load_review_inventory(&state).await;
    let latest_by_source = latest_review_head_by_source(&inventory);
    let current_head_sha =
        if let Some((repo, pr_number)) = parse_pr_diff_source(&session.diff_source) {
            fetch_current_pr_head_sha_for_review(&state, &repo, pr_number).await
        } else {
            None
        };
    let stale_review = is_review_stale(&session, &latest_by_source, current_head_sha.as_deref());
    let addressed_by_follow_up_comment_ids = infer_follow_up_addressed_comment_ids(
        &state,
        &session,
        &inventory,
        current_head_sha.as_deref(),
    )
    .await;
    let feedback_store = {
        let config = state.config.read().await.clone();
        crate::review::load_feedback_store(&config)
    };
    Ok(Json(build_api_review_session(
        apply_dynamic_review_state(session, &latest_by_source, current_head_sha.as_deref()),
        stale_review,
        &addressed_by_follow_up_comment_ids,
        Some(&feedback_store),
    )))
}

pub(crate) async fn get_review_forensics(
    Path(id): Path<String>,
) -> Result<Json<crate::forensics::ForensicsBundleManifest>, StatusCode> {
    crate::forensics::load_review_forensics_manifest(&id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::NOT_FOUND)
}

async fn fetch_current_pr_head_sha_for_review(
    state: &Arc<AppState>,
    repo: &str,
    pr_number: u32,
) -> Option<String> {
    let token = state
        .config
        .read()
        .await
        .github
        .token
        .clone()
        .filter(|token| !token.trim().is_empty());

    let token = token?;

    match fetch_github_pr_head_sha(&state.http_client, &token, repo, pr_number).await {
        Ok(head_sha) => Some(head_sha),
        Err(error) => {
            warn!(repo = %repo, pr_number, "Failed to fetch current PR head SHA: {error}");
            None
        }
    }
}

fn latest_follow_up_head_sha_from_inventory(
    session: &ReviewSession,
    inventory: &[ReviewSession],
) -> Option<String> {
    inventory
        .iter()
        .filter(|candidate| candidate.diff_source == session.diff_source)
        .filter(|candidate| candidate.started_at > session.started_at)
        .filter_map(|candidate| {
            candidate
                .github_head_sha
                .as_deref()
                .filter(|head_sha| Some(*head_sha) != session.github_head_sha.as_deref())
                .map(|head_sha| (candidate.started_at, head_sha.to_string()))
        })
        .max_by_key(|(started_at, _)| *started_at)
        .map(|(_, head_sha)| head_sha)
}

async fn infer_follow_up_addressed_comment_ids(
    state: &Arc<AppState>,
    session: &ReviewSession,
    inventory: &[ReviewSession],
    current_head_sha: Option<&str>,
) -> std::collections::HashSet<String> {
    let Some((repo, _)) = parse_pr_diff_source(&session.diff_source) else {
        return std::collections::HashSet::new();
    };

    let Some(reviewed_head_sha) = session.github_head_sha.as_deref() else {
        return std::collections::HashSet::new();
    };

    if !session
        .comments
        .iter()
        .any(|comment| comment.status == crate::core::comment::CommentStatus::Open)
    {
        return std::collections::HashSet::new();
    }

    let comparison_head_sha = current_head_sha
        .filter(|head_sha| *head_sha != reviewed_head_sha)
        .map(str::to_owned)
        .or_else(|| latest_follow_up_head_sha_from_inventory(session, inventory));

    let Some(comparison_head_sha) = comparison_head_sha else {
        return std::collections::HashSet::new();
    };

    let token = state
        .config
        .read()
        .await
        .github
        .token
        .clone()
        .filter(|token| !token.trim().is_empty());
    let Some(token) = token else {
        return std::collections::HashSet::new();
    };

    let compare_url = format!(
        "https://api.github.com/repos/{repo}/compare/{reviewed_head_sha}...{comparison_head_sha}",
    );
    let diff_content = match github_api_get_diff(&state.http_client, &token, &compare_url).await {
        Ok(diff_content) => diff_content,
        Err(error) => {
            warn!(
                review_id = %session.id,
                repo = %repo,
                reviewed_head_sha,
                comparison_head_sha,
                "Failed to fetch follow-up compare diff: {error}"
            );
            return std::collections::HashSet::new();
        }
    };

    let follow_up_diffs = match crate::core::DiffParser::parse_unified_diff(&diff_content) {
        Ok(follow_up_diffs) => follow_up_diffs,
        Err(error) => {
            warn!(
                review_id = %session.id,
                repo = %repo,
                reviewed_head_sha,
                comparison_head_sha,
                "Failed to parse follow-up compare diff: {error}"
            );
            return std::collections::HashSet::new();
        }
    };

    crate::core::comment::infer_addressed_by_follow_up_comments(&session.comments, &follow_up_diffs)
}

pub(crate) async fn list_reviews(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListReviewsParams>,
) -> Json<Vec<ReviewListItem>> {
    let page = params.page.unwrap_or(1).clamp(1, 10_000);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 100);

    let mut reviews = load_review_inventory(&state).await;
    let latest_by_source = latest_review_head_by_source(&reviews);
    reviews = reviews
        .into_iter()
        .map(|session| apply_dynamic_review_state(session, &latest_by_source, None))
        .collect();
    reviews.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    let start = (page - 1).saturating_mul(per_page);
    let list = if start < reviews.len() {
        let end = reviews.len().min(start.saturating_add(per_page));
        reviews[start..end]
            .iter()
            .map(ReviewListItem::from_session)
            .collect()
    } else {
        Vec::new()
    };

    Json(list)
}

pub(crate) async fn load_review_session_for_update(
    state: &Arc<AppState>,
    review_id: &str,
) -> Result<ReviewSession, StatusCode> {
    {
        let reviews = state.reviews.read().await;
        if let Some(session) = reviews.get(review_id) {
            return Ok(session.clone());
        }
    }

    match state.storage.get_review(review_id).await {
        Ok(Some(session)) => Ok(session),
        _ => Err(StatusCode::NOT_FOUND),
    }
}

pub(crate) async fn persist_updated_review_session(state: &Arc<AppState>, session: ReviewSession) {
    let review_id = session.id.clone();
    {
        let mut reviews = state.reviews.write().await;
        reviews.insert(review_id.clone(), session.clone());
    }

    AppState::save_reviews_async(state);

    if let Err(err) = state.storage.save_review(&session).await {
        warn!(
            review_id = %review_id,
            "Failed to persist updated review session: {}",
            err
        );
    }
}

pub(crate) async fn delete_review(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Remove from in-memory state
    {
        let mut reviews = state.reviews.write().await;
        reviews.remove(&id);
    }
    // Remove from storage backend
    if let Err(e) = state.storage.delete_review(&id).await {
        warn!("Failed to delete review {} from storage: {}", id, e);
    }
    Ok(Json(serde_json::json!({ "ok": true, "deleted": id })))
}

#[derive(Deserialize)]
pub(crate) struct PruneParams {
    pub max_age_days: Option<i64>,
    pub max_count: Option<usize>,
}

pub(crate) async fn prune_reviews(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PruneParams>,
) -> Json<serde_json::Value> {
    let retention = { state.config.read().await.retention.clone() };
    let max_age_secs = params
        .max_age_days
        .unwrap_or(retention.review_max_age_days)
        .max(1)
        * 86_400;
    let max_count = params
        .max_count
        .unwrap_or(retention.review_max_count)
        .max(1);

    match AppState::prune_reviews_with_limits(&state, max_age_secs, max_count).await {
        Ok(pruned) => {
            info!("Pruned {} old reviews", pruned);
            Json(serde_json::json!({ "ok": true, "pruned": pruned }))
        }
        Err(e) => {
            warn!("Prune failed: {}", e);
            Json(serde_json::json!({ "ok": false, "error": e.to_string() }))
        }
    }
}

pub(crate) async fn submit_feedback(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<FeedbackRequest>,
) -> Result<Json<FeedbackResponse>, StatusCode> {
    // Validate action
    if request.action != "accept" && request.action != "reject" {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut session = load_review_session_for_update(&state, &id).await?;

    let is_accepted = request.action == "accept";
    let (
        semantic_comment,
        github_repo,
        comment_content,
        comment_category,
        file_patterns,
        primary_file_pattern,
        feedback_changed,
    ) = {
        let comment = session
            .comments
            .iter_mut()
            .find(|c| c.id == request.comment_id)
            .ok_or(StatusCode::NOT_FOUND)?;

        // Capture comment data for convention store before mutating
        let semantic_comment = comment.clone();
        let comment_content = comment.content.clone();
        let comment_category = comment.category.to_string();
        let github_repo = parse_pr_diff_source(&session.diff_source).map(|(repo, _)| repo);
        let file_patterns = crate::review::derive_file_patterns(&comment.file_path);
        let primary_file_pattern = file_patterns.first().cloned();
        let feedback_changed = comment.feedback.as_deref() != Some(request.action.as_str());
        if feedback_changed {
            comment.feedback = Some(request.action.clone());
        }

        (
            semantic_comment,
            github_repo,
            comment_content,
            comment_category,
            file_patterns,
            primary_file_pattern,
            feedback_changed,
        )
    };

    let mut replay_capture_session = None;
    if feedback_changed {
        replay_capture_session = Some(session.clone());
        {
            let mut reviews = state.reviews.write().await;
            reviews.insert(id.clone(), session);
        }
        AppState::save_reviews_async(&state);

        if let Err(err) = state
            .storage
            .update_comment_feedback(
                &id,
                &request.comment_id,
                if is_accepted { "accept" } else { "reject" },
            )
            .await
        {
            warn!(
                review_id = %id,
                comment_id = %request.comment_id,
                "Failed to persist comment feedback: {}",
                err
            );
        }
    }

    if let Some(session) = replay_capture_session.as_ref() {
        if let Err(error) = crate::production_replay::record_review_feedback_fixture(session).await
        {
            warn!(review_id = %id, "Failed to update production replay fixture pack: {error}");
        }
    }

    // Record enhanced feedback pattern stats and explanation guidance.
    if feedback_changed || request.explanation.is_some() {
        let config = state.config.read().await.clone();
        let feedback_updated_at = chrono::Utc::now().to_rfc3339();
        let trust_context = resolve_feedback_trust_context(
            &state.http_client,
            &config,
            github_repo.as_deref(),
            request.github_login.as_deref(),
            request.github_role.as_deref(),
        )
        .await;
        let mut feedback_store = crate::review::load_feedback_store(&config);
        let feedback_signal_recorded = feedback_changed
            && crate::review::apply_comment_feedback_signal_with_weight(
                &mut feedback_store,
                &semantic_comment,
                is_accepted,
                trust_context.trust_weight,
            );
        let actor_recorded = (trust_context.github_login.is_some()
            || trust_context.github_role.is_some())
            && feedback_store.record_feedback_actor(
                &id,
                &request.comment_id,
                crate::review::FeedbackActorMetadata {
                    github_login: trust_context.github_login.clone(),
                    github_role: trust_context.github_role.clone(),
                    trust_weight: trust_context.trust_weight,
                    updated_at: feedback_updated_at.clone(),
                },
            );
        let explanation_recorded = if let Some(explanation) = request.explanation.as_deref() {
            feedback_store.record_feedback_explanation(
                &id,
                &semantic_comment,
                &file_patterns,
                &request.action,
                explanation,
                &feedback_updated_at,
            )
        } else if feedback_changed {
            feedback_store.clear_feedback_explanation(&id, &request.comment_id)
        } else {
            false
        };

        if feedback_signal_recorded || explanation_recorded || actor_recorded {
            let _ = crate::review::save_feedback_store(&config.feedback_path, &feedback_store);
        }

        if feedback_signal_recorded {
            let _ = crate::review::record_semantic_feedback_examples_with_weight(
                &config,
                std::slice::from_ref(&semantic_comment),
                is_accepted,
                trust_context.trust_weight,
            )
            .await;
        }
    }

    // Record in convention store for learned patterns.
    if feedback_changed {
        let config = state.config.read().await;
        let convention_path = config
            .convention_store_path
            .as_ref()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                dirs::data_local_dir().map(|d| d.join("diffscope").join("conventions.json"))
            });
        drop(config);

        if let Some(ref cpath) = convention_path {
            let json = std::fs::read_to_string(cpath).ok();
            let mut cstore = json
                .as_deref()
                .and_then(|j| ConventionStore::from_json(j).ok())
                .unwrap_or_default();
            let now = chrono::Utc::now().to_rfc3339();
            cstore.record_feedback(
                &comment_content,
                &comment_category,
                is_accepted,
                primary_file_pattern.as_deref(),
                &now,
            );
            if let Ok(out_json) = cstore.to_json() {
                if let Some(parent) = cpath.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(cpath, out_json);
            }
        }
    }

    Ok(Json(FeedbackResponse { ok: true }))
}

pub(crate) fn apply_comment_lifecycle_transition(
    comment: &mut crate::core::comment::Comment,
    next_status: CommentStatus,
    timestamp: i64,
) -> bool {
    if comment.status == next_status {
        return false;
    }

    comment.status = next_status;
    comment.resolved_at = match next_status {
        CommentStatus::Resolved => Some(timestamp),
        _ => None,
    };
    true
}

pub(crate) async fn update_comment_lifecycle(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<CommentLifecycleRequest>,
) -> Result<Json<CommentLifecycleResponse>, StatusCode> {
    let next_status = crate::core::comment::CommentStatus::from_api_str(&request.status)
        .ok_or(StatusCode::BAD_REQUEST)?;

    let mut session = load_review_session_for_update(&state, &id).await?;

    let (status_changed, dismissed_comment) = {
        let comment = session
            .comments
            .iter_mut()
            .find(|c| c.id == request.comment_id)
            .ok_or(StatusCode::NOT_FOUND)?;

        let changed = apply_comment_lifecycle_transition(comment, next_status, current_timestamp());
        let dismissed_comment = if changed && next_status == CommentStatus::Dismissed {
            Some(comment.clone())
        } else {
            None
        };

        (changed, dismissed_comment)
    };

    if status_changed {
        let previous_summary = session.summary.clone();
        session.summary = Some(CommentSynthesizer::inherit_review_state(
            CommentSynthesizer::generate_summary(&session.comments),
            previous_summary.as_ref(),
        ));
        persist_updated_review_session(&state, session).await;

        if let Some(comment) = dismissed_comment {
            let config = state.config.read().await.clone();
            let mut feedback_store = crate::review::load_feedback_store(&config);
            if crate::review::apply_comment_dismissal_signal(&mut feedback_store, &comment) {
                let _ = crate::review::save_feedback_store(&config.feedback_path, &feedback_store);
            }
        }
    }

    Ok(Json(CommentLifecycleResponse { ok: true }))
}
