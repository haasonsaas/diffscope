use std::fmt;
use std::sync::Arc;

use futures::FutureExt;

use crate::core::comment::{MergeReadiness, ReviewSummary};
use crate::core::dag::{execute_dag, DagNode, DagNodeExecutionHints, DagNodeSpec};
use crate::server::state::{AppState, ReviewSession, ReviewStatus};
use axum::http::StatusCode;
use tracing::debug;

use super::{
    build_fix_loop_replay_candidates, build_pr_fix_handoff_response, build_pr_fix_loop_response,
    build_rerun_pr_review_request, dispatch_pr_review, FixLoopProfile, FixLoopReplayCandidate,
    FixLoopStatus, FixLoopStopReason, PrFixHandoffResponse, PrFixLoopResponse,
    PrFixLoopResponseArgs, StartPrReviewRequest, StartReviewResponse,
};

#[derive(Debug, Clone)]
pub(crate) struct FixLoopDagSnapshot {
    pub repo: String,
    pub pr_number: u32,
    pub profile: FixLoopProfile,
    pub max_iterations: usize,
    pub replay_limit: usize,
    pub auto_start_review: bool,
    pub auto_rerun_stale: bool,
    pub completed_reviews: usize,
    pub current_head_sha: Option<String>,
    pub latest_review: Option<ReviewSession>,
    pub latest_completed_review: Option<ReviewSession>,
    pub latest_review_stale: bool,
    pub previous_summary: Option<ReviewSummary>,
    pub improvement_detected: Option<bool>,
    pub loop_telemetry: Option<crate::core::comment::FixLoopTelemetry>,
    pub stalled_iterations: usize,
}

impl FixLoopDagSnapshot {
    fn latest_summary_ref(&self) -> Result<&ReviewSummary, FixLoopDagError> {
        self.latest_completed_review
            .as_ref()
            .and_then(|review| review.summary.as_ref())
            .ok_or_else(|| {
                FixLoopDagError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Latest completed review is missing a readiness summary.",
                )
            })
    }

    fn latest_summary(&self) -> Option<ReviewSummary> {
        self.latest_completed_review
            .as_ref()
            .and_then(|review| review.summary.clone())
    }
}

struct FixLoopDagContext {
    snapshot: FixLoopDagSnapshot,
    plan: FixLoopDagPlan,
    triggered_review: Option<StartReviewResponse>,
    fix_handoff: Option<PrFixHandoffResponse>,
    replay_candidates: Vec<FixLoopReplayCandidate>,
    response: Option<PrFixLoopResponse>,
}

impl FixLoopDagContext {
    fn new(snapshot: FixLoopDagSnapshot, plan: FixLoopDagPlan) -> Self {
        Self {
            snapshot,
            plan,
            triggered_review: None,
            fix_handoff: None,
            replay_candidates: Vec::new(),
            response: None,
        }
    }

    fn into_response(self) -> Result<PrFixLoopResponse, FixLoopDagError> {
        self.response.ok_or_else(|| {
            FixLoopDagError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Fix-loop DAG did not produce a response.",
            )
        })
    }

    fn latest_completed_review(&self) -> Result<&ReviewSession, FixLoopDagError> {
        self.snapshot
            .latest_completed_review
            .as_ref()
            .ok_or_else(|| {
                FixLoopDagError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Fix-loop DAG expected a completed review but none was available.",
                )
            })
    }

    fn latest_review(&self) -> Result<&ReviewSession, FixLoopDagError> {
        self.snapshot.latest_review.as_ref().ok_or_else(|| {
            FixLoopDagError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Fix-loop DAG expected a latest review but none was available.",
            )
        })
    }

    fn triggered_review(&self) -> Result<&StartReviewResponse, FixLoopDagError> {
        self.triggered_review.as_ref().ok_or_else(|| {
            FixLoopDagError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Fix-loop DAG expected a triggered review but none was recorded.",
            )
        })
    }

    fn set_response(&mut self, response: PrFixLoopResponse) {
        self.response = Some(response);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixLoopDagPlan {
    ReviewPending,
    Failed,
    StartInitialReview,
    NeedsInitialReview,
    ExhaustedBeforeLatestHeadReview,
    StartStaleRerun,
    NeedsStaleReview,
    Converged,
    Exhausted,
    Stalled,
    NeedsFixes,
}

impl FixLoopDagPlan {
    fn requires_fix_artifacts(self) -> bool {
        matches!(
            self,
            Self::ExhaustedBeforeLatestHeadReview
                | Self::Exhausted
                | Self::Stalled
                | Self::NeedsFixes
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FixLoopDagStage {
    EvaluateReviewState,
    StartInitialReview,
    RerunStaleReview,
    BuildFixArtifacts,
    ReportReviewPending,
    ReportFailed,
    ReportNeedsReview,
    ReportConverged,
    ReportExhausted,
    ReportStalled,
    ReportNeedsFixes,
}

impl DagNode for FixLoopDagStage {
    fn name(&self) -> &'static str {
        match self {
            Self::EvaluateReviewState => "evaluate_review_state",
            Self::StartInitialReview => "start_initial_review",
            Self::RerunStaleReview => "rerun_stale_review",
            Self::BuildFixArtifacts => "build_fix_artifacts",
            Self::ReportReviewPending => "report_review_pending",
            Self::ReportFailed => "report_failed",
            Self::ReportNeedsReview => "report_needs_review",
            Self::ReportConverged => "report_converged",
            Self::ReportExhausted => "report_exhausted",
            Self::ReportStalled => "report_stalled",
            Self::ReportNeedsFixes => "report_needs_fixes",
        }
    }
}

#[derive(Debug)]
struct FixLoopDagError {
    status: StatusCode,
    message: String,
}

impl FixLoopDagError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl fmt::Display for FixLoopDagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FixLoopDagError {}

impl From<(StatusCode, String)> for FixLoopDagError {
    fn from((status, message): (StatusCode, String)) -> Self {
        Self { status, message }
    }
}

pub(crate) async fn execute_fix_loop_dag(
    state: &Arc<AppState>,
    snapshot: FixLoopDagSnapshot,
) -> std::result::Result<PrFixLoopResponse, (StatusCode, String)> {
    let plan = determine_fix_loop_plan(&snapshot).map_err(map_fix_loop_dag_error)?;
    let specs = build_fix_loop_specs(plan);
    let mut context = FixLoopDagContext::new(snapshot, plan);
    let state = Arc::clone(state);

    let records = execute_dag(&specs, &mut context, move |stage, context| {
        let state = Arc::clone(&state);
        async move {
            execute_fix_loop_stage(&state, stage, context)
                .await
                .map_err(anyhow::Error::new)
        }
        .boxed()
    })
    .await
    .map_err(map_fix_loop_dag_error)?;

    debug!(?plan, ?records, "Executed fix-loop DAG");
    context.into_response().map_err(map_fix_loop_dag_error)
}

fn map_fix_loop_dag_error(error: impl Into<anyhow::Error>) -> (StatusCode, String) {
    let error = error.into();
    if let Some(dag_error) = error.downcast_ref::<FixLoopDagError>() {
        return (dag_error.status, dag_error.message.clone());
    }
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Fix-loop DAG failed: {error}"),
    )
}

fn determine_fix_loop_plan(
    snapshot: &FixLoopDagSnapshot,
) -> Result<FixLoopDagPlan, FixLoopDagError> {
    if let Some(latest_review) = snapshot.latest_review.as_ref() {
        if matches!(
            latest_review.status,
            ReviewStatus::Pending | ReviewStatus::Running
        ) {
            return Ok(FixLoopDagPlan::ReviewPending);
        }

        if latest_review.status == ReviewStatus::Failed
            && snapshot
                .latest_completed_review
                .as_ref()
                .is_none_or(|completed| latest_review.started_at >= completed.started_at)
        {
            return Ok(FixLoopDagPlan::Failed);
        }
    }

    let Some(_) = snapshot.latest_completed_review.as_ref() else {
        return Ok(if snapshot.auto_start_review {
            FixLoopDagPlan::StartInitialReview
        } else {
            FixLoopDagPlan::NeedsInitialReview
        });
    };

    if snapshot.latest_review_stale {
        if snapshot.completed_reviews >= snapshot.max_iterations {
            return Ok(FixLoopDagPlan::ExhaustedBeforeLatestHeadReview);
        }
        if snapshot.auto_rerun_stale {
            return Ok(FixLoopDagPlan::StartStaleRerun);
        }
        return Ok(FixLoopDagPlan::NeedsStaleReview);
    }

    let latest_summary = snapshot.latest_summary_ref()?;
    if latest_summary.merge_readiness == MergeReadiness::Ready
        && latest_summary.open_blockers == 0
        && latest_summary.open_comments == 0
    {
        return Ok(FixLoopDagPlan::Converged);
    }

    if snapshot.completed_reviews >= snapshot.max_iterations {
        return Ok(FixLoopDagPlan::Exhausted);
    }

    if snapshot.stalled_iterations >= 2 {
        return Ok(FixLoopDagPlan::Stalled);
    }

    Ok(FixLoopDagPlan::NeedsFixes)
}

fn stage_hints(stage: FixLoopDagStage) -> DagNodeExecutionHints {
    match stage {
        FixLoopDagStage::StartInitialReview | FixLoopDagStage::RerunStaleReview => {
            DagNodeExecutionHints {
                parallelizable: false,
                retryable: true,
                timeout_ms: None,
                side_effects: true,
                subgraph: Some("review_pipeline".to_string()),
            }
        }
        _ => DagNodeExecutionHints {
            parallelizable: false,
            retryable: true,
            timeout_ms: None,
            side_effects: false,
            subgraph: None,
        },
    }
}

fn build_fix_loop_specs(plan: FixLoopDagPlan) -> Vec<DagNodeSpec<FixLoopDagStage>> {
    vec![
        spec(FixLoopDagStage::EvaluateReviewState, vec![], true),
        spec(
            FixLoopDagStage::StartInitialReview,
            vec![FixLoopDagStage::EvaluateReviewState],
            plan == FixLoopDagPlan::StartInitialReview,
        ),
        spec(
            FixLoopDagStage::RerunStaleReview,
            vec![FixLoopDagStage::EvaluateReviewState],
            plan == FixLoopDagPlan::StartStaleRerun,
        ),
        spec(
            FixLoopDagStage::BuildFixArtifacts,
            vec![FixLoopDagStage::EvaluateReviewState],
            plan.requires_fix_artifacts(),
        ),
        spec(
            FixLoopDagStage::ReportReviewPending,
            vec![
                FixLoopDagStage::EvaluateReviewState,
                FixLoopDagStage::StartInitialReview,
                FixLoopDagStage::RerunStaleReview,
            ],
            matches!(
                plan,
                FixLoopDagPlan::ReviewPending
                    | FixLoopDagPlan::StartInitialReview
                    | FixLoopDagPlan::StartStaleRerun
            ),
        ),
        spec(
            FixLoopDagStage::ReportFailed,
            vec![FixLoopDagStage::EvaluateReviewState],
            plan == FixLoopDagPlan::Failed,
        ),
        spec(
            FixLoopDagStage::ReportNeedsReview,
            vec![FixLoopDagStage::EvaluateReviewState],
            matches!(
                plan,
                FixLoopDagPlan::NeedsInitialReview | FixLoopDagPlan::NeedsStaleReview
            ),
        ),
        spec(
            FixLoopDagStage::ReportConverged,
            vec![FixLoopDagStage::EvaluateReviewState],
            plan == FixLoopDagPlan::Converged,
        ),
        spec(
            FixLoopDagStage::ReportExhausted,
            vec![
                FixLoopDagStage::EvaluateReviewState,
                FixLoopDagStage::BuildFixArtifacts,
            ],
            matches!(
                plan,
                FixLoopDagPlan::ExhaustedBeforeLatestHeadReview | FixLoopDagPlan::Exhausted
            ),
        ),
        spec(
            FixLoopDagStage::ReportStalled,
            vec![
                FixLoopDagStage::EvaluateReviewState,
                FixLoopDagStage::BuildFixArtifacts,
            ],
            plan == FixLoopDagPlan::Stalled,
        ),
        spec(
            FixLoopDagStage::ReportNeedsFixes,
            vec![
                FixLoopDagStage::EvaluateReviewState,
                FixLoopDagStage::BuildFixArtifacts,
            ],
            plan == FixLoopDagPlan::NeedsFixes,
        ),
    ]
}

fn spec(
    id: FixLoopDagStage,
    dependencies: Vec<FixLoopDagStage>,
    enabled: bool,
) -> DagNodeSpec<FixLoopDagStage> {
    DagNodeSpec {
        id,
        dependencies,
        hints: stage_hints(id),
        enabled,
    }
}

async fn execute_fix_loop_stage(
    state: &Arc<AppState>,
    stage: FixLoopDagStage,
    context: &mut FixLoopDagContext,
) -> Result<(), FixLoopDagError> {
    match stage {
        FixLoopDagStage::EvaluateReviewState => Ok(()),
        FixLoopDagStage::StartInitialReview => {
            if context.plan == FixLoopDagPlan::StartInitialReview {
                let started = dispatch_pr_review(
                    state,
                    StartPrReviewRequest {
                        repo: context.snapshot.repo.clone(),
                        pr_number: context.snapshot.pr_number,
                        post_results: false,
                    },
                )
                .await
                .map_err(FixLoopDagError::from)?;
                context.triggered_review = Some(started);
            }
            Ok(())
        }
        FixLoopDagStage::RerunStaleReview => {
            if context.plan == FixLoopDagPlan::StartStaleRerun {
                let request =
                    build_rerun_pr_review_request(context.latest_completed_review()?, Some(false))
                        .map_err(FixLoopDagError::from)?;
                let started = dispatch_pr_review(state, request)
                    .await
                    .map_err(FixLoopDagError::from)?;
                context.triggered_review = Some(started);
            }
            Ok(())
        }
        FixLoopDagStage::BuildFixArtifacts => {
            if context.plan.requires_fix_artifacts() {
                let latest_review = context.latest_completed_review()?.clone();
                context.fix_handoff = Some(build_pr_fix_handoff_response(
                    &context.snapshot.repo,
                    context.snapshot.pr_number,
                    Some(&latest_review),
                    false,
                ));
                context.replay_candidates = build_fix_loop_replay_candidates(
                    &context.snapshot.repo,
                    context.snapshot.pr_number,
                    &latest_review,
                    context.snapshot.replay_limit,
                );
            }
            Ok(())
        }
        FixLoopDagStage::ReportReviewPending => {
            if matches!(
                context.plan,
                FixLoopDagPlan::ReviewPending
                    | FixLoopDagPlan::StartInitialReview
                    | FixLoopDagPlan::StartStaleRerun
            ) {
                context.set_response(build_review_pending_response(context)?);
            }
            Ok(())
        }
        FixLoopDagStage::ReportFailed => {
            if context.plan == FixLoopDagPlan::Failed {
                context.set_response(build_failed_response(context)?);
            }
            Ok(())
        }
        FixLoopDagStage::ReportNeedsReview => {
            if matches!(
                context.plan,
                FixLoopDagPlan::NeedsInitialReview | FixLoopDagPlan::NeedsStaleReview
            ) {
                context.set_response(build_needs_review_response(context)?);
            }
            Ok(())
        }
        FixLoopDagStage::ReportConverged => {
            if context.plan == FixLoopDagPlan::Converged {
                context.set_response(build_converged_response(context)?);
            }
            Ok(())
        }
        FixLoopDagStage::ReportExhausted => {
            if matches!(
                context.plan,
                FixLoopDagPlan::ExhaustedBeforeLatestHeadReview | FixLoopDagPlan::Exhausted
            ) {
                context.set_response(build_exhausted_response(context)?);
            }
            Ok(())
        }
        FixLoopDagStage::ReportStalled => {
            if context.plan == FixLoopDagPlan::Stalled {
                context.set_response(build_stalled_response(context)?);
            }
            Ok(())
        }
        FixLoopDagStage::ReportNeedsFixes => {
            if context.plan == FixLoopDagPlan::NeedsFixes {
                context.set_response(build_needs_fixes_response(context)?);
            }
            Ok(())
        }
    }
}

fn build_review_pending_response(
    context: &FixLoopDagContext,
) -> Result<PrFixLoopResponse, FixLoopDagError> {
    match context.plan {
        FixLoopDagPlan::ReviewPending => {
            let latest_review = context.latest_review()?;
            Ok(build_pr_fix_loop_response(PrFixLoopResponseArgs {
                repo: context.snapshot.repo.clone(),
                pr_number: context.snapshot.pr_number,
                profile: context.snapshot.profile,
                max_iterations: context.snapshot.max_iterations,
                replay_limit: context.snapshot.replay_limit,
                auto_start_review: context.snapshot.auto_start_review,
                auto_rerun_stale: context.snapshot.auto_rerun_stale,
                completed_reviews: context.snapshot.completed_reviews,
                status: FixLoopStatus::ReviewPending,
                next_action: "wait_for_review".to_string(),
                status_message: format!(
                    "Waiting for DiffScope review '{}' to finish before continuing the fix loop.",
                    latest_review.id
                ),
                latest_review_id: Some(latest_review.id.clone()),
                latest_review_status: Some(latest_review.status.clone()),
                triggered_review_id: None,
                current_head_sha: context.snapshot.current_head_sha.clone(),
                reviewed_head_sha: latest_review.github_head_sha.clone(),
                latest_review_stale: false,
                summary: None,
                previous_summary: context.snapshot.previous_summary.clone(),
                improvement_detected: None,
                loop_telemetry: context.snapshot.loop_telemetry.clone(),
                stalled_iterations: context.snapshot.stalled_iterations,
                stop_reason: None,
                replay_candidates: Vec::new(),
                fix_handoff: None,
            }))
        }
        FixLoopDagPlan::StartInitialReview => {
            let started = context.triggered_review()?;
            Ok(build_pr_fix_loop_response(PrFixLoopResponseArgs {
                repo: context.snapshot.repo.clone(),
                pr_number: context.snapshot.pr_number,
                profile: context.snapshot.profile,
                max_iterations: context.snapshot.max_iterations,
                replay_limit: context.snapshot.replay_limit,
                auto_start_review: context.snapshot.auto_start_review,
                auto_rerun_stale: context.snapshot.auto_rerun_stale,
                completed_reviews: 0,
                status: FixLoopStatus::ReviewPending,
                next_action: "wait_for_review".to_string(),
                status_message: format!(
                    "Started DiffScope review '{}' to begin the fix loop.",
                    started.id
                ),
                latest_review_id: Some(started.id.clone()),
                latest_review_status: Some(started.status.clone()),
                triggered_review_id: Some(started.id.clone()),
                current_head_sha: context.snapshot.current_head_sha.clone(),
                reviewed_head_sha: context.snapshot.current_head_sha.clone(),
                latest_review_stale: false,
                summary: None,
                previous_summary: None,
                improvement_detected: None,
                loop_telemetry: None,
                stalled_iterations: 0,
                stop_reason: None,
                replay_candidates: Vec::new(),
                fix_handoff: None,
            }))
        }
        FixLoopDagPlan::StartStaleRerun => {
            let started = context.triggered_review()?;
            Ok(build_pr_fix_loop_response(PrFixLoopResponseArgs {
                repo: context.snapshot.repo.clone(),
                pr_number: context.snapshot.pr_number,
                profile: context.snapshot.profile,
                max_iterations: context.snapshot.max_iterations,
                replay_limit: context.snapshot.replay_limit,
                auto_start_review: context.snapshot.auto_start_review,
                auto_rerun_stale: context.snapshot.auto_rerun_stale,
                completed_reviews: context.snapshot.completed_reviews,
                status: FixLoopStatus::ReviewPending,
                next_action: "wait_for_review".to_string(),
                status_message: format!(
                    "Started DiffScope rerun '{}' for the latest PR head.",
                    started.id
                ),
                latest_review_id: Some(started.id.clone()),
                latest_review_status: Some(started.status.clone()),
                triggered_review_id: Some(started.id.clone()),
                current_head_sha: context.snapshot.current_head_sha.clone(),
                reviewed_head_sha: context.snapshot.current_head_sha.clone(),
                latest_review_stale: false,
                summary: None,
                previous_summary: context.snapshot.previous_summary.clone(),
                improvement_detected: None,
                loop_telemetry: context.snapshot.loop_telemetry.clone(),
                stalled_iterations: context.snapshot.stalled_iterations,
                stop_reason: None,
                replay_candidates: Vec::new(),
                fix_handoff: None,
            }))
        }
        _ => Err(FixLoopDagError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected fix-loop plan for review_pending response.",
        )),
    }
}

fn build_failed_response(
    context: &FixLoopDagContext,
) -> Result<PrFixLoopResponse, FixLoopDagError> {
    let latest_review = context.latest_review()?;
    Ok(build_pr_fix_loop_response(PrFixLoopResponseArgs {
        repo: context.snapshot.repo.clone(),
        pr_number: context.snapshot.pr_number,
        profile: context.snapshot.profile,
        max_iterations: context.snapshot.max_iterations,
        replay_limit: context.snapshot.replay_limit,
        auto_start_review: context.snapshot.auto_start_review,
        auto_rerun_stale: context.snapshot.auto_rerun_stale,
        completed_reviews: context.snapshot.completed_reviews,
        status: FixLoopStatus::Failed,
        next_action: "stop".to_string(),
        status_message: latest_review
            .error
            .clone()
            .unwrap_or_else(|| "The latest DiffScope review failed.".to_string()),
        latest_review_id: Some(latest_review.id.clone()),
        latest_review_status: Some(latest_review.status.clone()),
        triggered_review_id: None,
        current_head_sha: context.snapshot.current_head_sha.clone(),
        reviewed_head_sha: latest_review.github_head_sha.clone(),
        latest_review_stale: false,
        summary: None,
        previous_summary: context.snapshot.previous_summary.clone(),
        improvement_detected: None,
        loop_telemetry: context.snapshot.loop_telemetry.clone(),
        stalled_iterations: context.snapshot.stalled_iterations,
        stop_reason: Some(FixLoopStopReason::ReviewFailed),
        replay_candidates: Vec::new(),
        fix_handoff: None,
    }))
}

fn build_needs_review_response(
    context: &FixLoopDagContext,
) -> Result<PrFixLoopResponse, FixLoopDagError> {
    match context.plan {
        FixLoopDagPlan::NeedsInitialReview => Ok(build_pr_fix_loop_response(
            PrFixLoopResponseArgs {
                repo: context.snapshot.repo.clone(),
                pr_number: context.snapshot.pr_number,
                profile: context.snapshot.profile,
                max_iterations: context.snapshot.max_iterations,
                replay_limit: context.snapshot.replay_limit,
                auto_start_review: context.snapshot.auto_start_review,
                auto_rerun_stale: context.snapshot.auto_rerun_stale,
                completed_reviews: 0,
                status: FixLoopStatus::NeedsReview,
                next_action: "start_review".to_string(),
                status_message:
                    "No completed DiffScope review exists for this PR. Start a review to begin the fix loop."
                        .to_string(),
                latest_review_id: None,
                latest_review_status: None,
                triggered_review_id: None,
                current_head_sha: context.snapshot.current_head_sha.clone(),
                reviewed_head_sha: None,
                latest_review_stale: false,
                summary: None,
                previous_summary: None,
                improvement_detected: None,
                loop_telemetry: None,
                stalled_iterations: 0,
                stop_reason: None,
                replay_candidates: Vec::new(),
                fix_handoff: None,
            },
        )),
        FixLoopDagPlan::NeedsStaleReview => {
            let latest_review = context.latest_completed_review()?;
            Ok(build_pr_fix_loop_response(PrFixLoopResponseArgs {
                repo: context.snapshot.repo.clone(),
                pr_number: context.snapshot.pr_number,
                profile: context.snapshot.profile,
                max_iterations: context.snapshot.max_iterations,
                replay_limit: context.snapshot.replay_limit,
                auto_start_review: context.snapshot.auto_start_review,
                auto_rerun_stale: context.snapshot.auto_rerun_stale,
                completed_reviews: context.snapshot.completed_reviews,
                status: FixLoopStatus::NeedsReview,
                next_action: "rerun_review".to_string(),
                status_message:
                    "The latest DiffScope review is stale against the current PR head. Rerun the review before applying more fixes."
                        .to_string(),
                latest_review_id: Some(latest_review.id.clone()),
                latest_review_status: Some(latest_review.status.clone()),
                triggered_review_id: None,
                current_head_sha: context.snapshot.current_head_sha.clone(),
                reviewed_head_sha: latest_review.github_head_sha.clone(),
                latest_review_stale: true,
                summary: context.snapshot.latest_summary(),
                previous_summary: context.snapshot.previous_summary.clone(),
                improvement_detected: context.snapshot.improvement_detected,
                loop_telemetry: context.snapshot.loop_telemetry.clone(),
                stalled_iterations: context.snapshot.stalled_iterations,
                stop_reason: None,
                replay_candidates: Vec::new(),
                fix_handoff: None,
            }))
        }
        _ => Err(FixLoopDagError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected fix-loop plan for needs_review response.",
        )),
    }
}

fn build_converged_response(
    context: &FixLoopDagContext,
) -> Result<PrFixLoopResponse, FixLoopDagError> {
    let latest_review = context.latest_completed_review()?;
    Ok(build_pr_fix_loop_response(PrFixLoopResponseArgs {
        repo: context.snapshot.repo.clone(),
        pr_number: context.snapshot.pr_number,
        profile: context.snapshot.profile,
        max_iterations: context.snapshot.max_iterations,
        replay_limit: context.snapshot.replay_limit,
        auto_start_review: context.snapshot.auto_start_review,
        auto_rerun_stale: context.snapshot.auto_rerun_stale,
        completed_reviews: context.snapshot.completed_reviews,
        status: FixLoopStatus::Converged,
        next_action: "stop".to_string(),
        status_message: "PR is ready and the fix loop converged with no unresolved findings."
            .to_string(),
        latest_review_id: Some(latest_review.id.clone()),
        latest_review_status: Some(latest_review.status.clone()),
        triggered_review_id: None,
        current_head_sha: context.snapshot.current_head_sha.clone(),
        reviewed_head_sha: latest_review.github_head_sha.clone(),
        latest_review_stale: false,
        summary: context.snapshot.latest_summary(),
        previous_summary: context.snapshot.previous_summary.clone(),
        improvement_detected: context.snapshot.improvement_detected,
        loop_telemetry: context.snapshot.loop_telemetry.clone(),
        stalled_iterations: context.snapshot.stalled_iterations,
        stop_reason: Some(FixLoopStopReason::Ready),
        replay_candidates: Vec::new(),
        fix_handoff: None,
    }))
}

fn build_exhausted_response(
    context: &FixLoopDagContext,
) -> Result<PrFixLoopResponse, FixLoopDagError> {
    let latest_review = context.latest_completed_review()?;
    let status_message = match context.plan {
        FixLoopDagPlan::ExhaustedBeforeLatestHeadReview => {
            "Fix loop budget exhausted before DiffScope could review the latest PR head."
                .to_string()
        }
        FixLoopDagPlan::Exhausted => format!(
            "Fix loop reached its review budget of {} completed review(s) with blockers still open.",
            context.snapshot.max_iterations
        ),
        _ => {
            return Err(FixLoopDagError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unexpected fix-loop plan for exhausted response.",
            ));
        }
    };

    Ok(build_pr_fix_loop_response(PrFixLoopResponseArgs {
        repo: context.snapshot.repo.clone(),
        pr_number: context.snapshot.pr_number,
        profile: context.snapshot.profile,
        max_iterations: context.snapshot.max_iterations,
        replay_limit: context.snapshot.replay_limit,
        auto_start_review: context.snapshot.auto_start_review,
        auto_rerun_stale: context.snapshot.auto_rerun_stale,
        completed_reviews: context.snapshot.completed_reviews,
        status: FixLoopStatus::Exhausted,
        next_action: "stop".to_string(),
        status_message,
        latest_review_id: Some(latest_review.id.clone()),
        latest_review_status: Some(latest_review.status.clone()),
        triggered_review_id: None,
        current_head_sha: context.snapshot.current_head_sha.clone(),
        reviewed_head_sha: latest_review.github_head_sha.clone(),
        latest_review_stale: context.snapshot.latest_review_stale,
        summary: context.snapshot.latest_summary(),
        previous_summary: context.snapshot.previous_summary.clone(),
        improvement_detected: context.snapshot.improvement_detected,
        loop_telemetry: context.snapshot.loop_telemetry.clone(),
        stalled_iterations: context.snapshot.stalled_iterations,
        stop_reason: Some(FixLoopStopReason::MaxIterations),
        replay_candidates: context.replay_candidates.clone(),
        fix_handoff: context.fix_handoff.clone(),
    }))
}

fn build_stalled_response(
    context: &FixLoopDagContext,
) -> Result<PrFixLoopResponse, FixLoopDagError> {
    let latest_review = context.latest_completed_review()?;
    Ok(build_pr_fix_loop_response(PrFixLoopResponseArgs {
        repo: context.snapshot.repo.clone(),
        pr_number: context.snapshot.pr_number,
        profile: context.snapshot.profile,
        max_iterations: context.snapshot.max_iterations,
        replay_limit: context.snapshot.replay_limit,
        auto_start_review: context.snapshot.auto_start_review,
        auto_rerun_stale: context.snapshot.auto_rerun_stale,
        completed_reviews: context.snapshot.completed_reviews,
        status: FixLoopStatus::Stalled,
        next_action: "stop".to_string(),
        status_message:
            "Fix loop stopped after two consecutive review iterations showed no improvement."
                .to_string(),
        latest_review_id: Some(latest_review.id.clone()),
        latest_review_status: Some(latest_review.status.clone()),
        triggered_review_id: None,
        current_head_sha: context.snapshot.current_head_sha.clone(),
        reviewed_head_sha: latest_review.github_head_sha.clone(),
        latest_review_stale: false,
        summary: context.snapshot.latest_summary(),
        previous_summary: context.snapshot.previous_summary.clone(),
        improvement_detected: context.snapshot.improvement_detected,
        loop_telemetry: context.snapshot.loop_telemetry.clone(),
        stalled_iterations: context.snapshot.stalled_iterations,
        stop_reason: Some(FixLoopStopReason::NoImprovement),
        replay_candidates: context.replay_candidates.clone(),
        fix_handoff: context.fix_handoff.clone(),
    }))
}

fn build_needs_fixes_response(
    context: &FixLoopDagContext,
) -> Result<PrFixLoopResponse, FixLoopDagError> {
    let latest_review = context.latest_completed_review()?;
    Ok(build_pr_fix_loop_response(PrFixLoopResponseArgs {
        repo: context.snapshot.repo.clone(),
        pr_number: context.snapshot.pr_number,
        profile: context.snapshot.profile,
        max_iterations: context.snapshot.max_iterations,
        replay_limit: context.snapshot.replay_limit,
        auto_start_review: context.snapshot.auto_start_review,
        auto_rerun_stale: context.snapshot.auto_rerun_stale,
        completed_reviews: context.snapshot.completed_reviews,
        status: FixLoopStatus::NeedsFixes,
        next_action: "apply_fixes".to_string(),
        status_message: "Apply the unresolved fixes, push the changes, and call run_fix_until_clean again to assess the new head."
            .to_string(),
        latest_review_id: Some(latest_review.id.clone()),
        latest_review_status: Some(latest_review.status.clone()),
        triggered_review_id: None,
        current_head_sha: context.snapshot.current_head_sha.clone(),
        reviewed_head_sha: latest_review.github_head_sha.clone(),
        latest_review_stale: false,
        summary: context.snapshot.latest_summary(),
        previous_summary: context.snapshot.previous_summary.clone(),
        improvement_detected: context.snapshot.improvement_detected,
        loop_telemetry: context.snapshot.loop_telemetry.clone(),
        stalled_iterations: context.snapshot.stalled_iterations,
        stop_reason: None,
        replay_candidates: context.replay_candidates.clone(),
        fix_handoff: context.fix_handoff.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot() -> FixLoopDagSnapshot {
        FixLoopDagSnapshot {
            repo: "owner/repo".to_string(),
            pr_number: 42,
            profile: FixLoopProfile::HighAutonomyFixer,
            max_iterations: 4,
            replay_limit: 2,
            auto_start_review: true,
            auto_rerun_stale: true,
            completed_reviews: 0,
            current_head_sha: Some("sha-current".to_string()),
            latest_review: None,
            latest_completed_review: None,
            latest_review_stale: false,
            previous_summary: None,
            improvement_detected: None,
            loop_telemetry: None,
            stalled_iterations: 0,
        }
    }

    fn make_completed_review(
        review_id: &str,
        started_at: i64,
        head_sha: &str,
        blockers: usize,
        comments: usize,
        readiness: MergeReadiness,
    ) -> ReviewSession {
        let mut summary = crate::core::CommentSynthesizer::generate_summary(&[]);
        summary.open_blockers = blockers;
        summary.open_comments = comments;
        summary.merge_readiness = readiness;

        ReviewSession {
            id: review_id.to_string(),
            status: ReviewStatus::Complete,
            diff_source: "pr:owner/repo#42".to_string(),
            github_head_sha: Some(head_sha.to_string()),
            github_post_results_requested: Some(false),
            started_at,
            completed_at: Some(started_at + 1),
            comments: Vec::new(),
            summary: Some(summary),
            files_reviewed: 1,
            error: None,
            pr_summary_text: None,
            diff_content: None,
            event: None,
            progress: None,
        }
    }

    #[test]
    fn determine_fix_loop_plan_starts_initial_review_when_allowed() {
        let snapshot = make_snapshot();

        assert_eq!(
            determine_fix_loop_plan(&snapshot).unwrap(),
            FixLoopDagPlan::StartInitialReview
        );
    }

    #[test]
    fn determine_fix_loop_plan_requests_stale_rerun_before_fixes() {
        let mut snapshot = make_snapshot();
        snapshot.completed_reviews = 2;
        snapshot.latest_review_stale = true;
        snapshot.latest_completed_review = Some(make_completed_review(
            "review-1",
            10,
            "sha-old",
            2,
            2,
            MergeReadiness::NeedsAttention,
        ));

        assert_eq!(
            determine_fix_loop_plan(&snapshot).unwrap(),
            FixLoopDagPlan::StartStaleRerun
        );
    }

    #[test]
    fn determine_fix_loop_plan_falls_back_to_needs_fixes_with_open_blockers() {
        let mut snapshot = make_snapshot();
        snapshot.completed_reviews = 1;
        snapshot.latest_completed_review = Some(make_completed_review(
            "review-1",
            10,
            "sha-current",
            1,
            1,
            MergeReadiness::NeedsAttention,
        ));

        assert_eq!(
            determine_fix_loop_plan(&snapshot).unwrap(),
            FixLoopDagPlan::NeedsFixes
        );
    }

    #[test]
    fn build_fix_loop_specs_marks_side_effect_nodes_and_artifact_stage() {
        let specs = build_fix_loop_specs(FixLoopDagPlan::StartStaleRerun);

        assert!(specs
            .iter()
            .find(|spec| spec.id == FixLoopDagStage::RerunStaleReview)
            .is_some_and(|spec| spec.enabled && spec.hints.side_effects));
        assert!(specs
            .iter()
            .find(|spec| spec.id == FixLoopDagStage::BuildFixArtifacts)
            .is_some_and(|spec| !spec.enabled));
    }
}
