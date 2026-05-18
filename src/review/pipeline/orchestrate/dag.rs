use anyhow::Result;
use futures::FutureExt;
use std::path::Path;
use tracing::debug;

use crate::config;
use crate::core::dag::{
    describe_dag, execute_dag, DagExecutionTrace, DagGraphContract, DagNode, DagNodeContract,
    DagNodeExecutionHints, DagNodeKind, DagNodeSpec,
};

use super::super::contracts::{ExecutionSummary, PreparedReviewJobs, ReviewExecutionContext};
use super::super::execution::execute_review_jobs;
use super::super::postprocess::run_postprocess;
use super::super::prepare::prepare_file_review_jobs;
use super::super::services::PipelineServices;
use super::super::session::ReviewSession;
use super::super::types::{ProgressCallback, ReviewResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ReviewPipelineStage {
    InitializeServices,
    BuildSession,
    PrepareJobs,
    ExecuteJobs,
    Postprocess,
}

impl DagNode for ReviewPipelineStage {
    fn name(&self) -> &'static str {
        match self {
            Self::InitializeServices => "initialize_services",
            Self::BuildSession => "build_session",
            Self::PrepareJobs => "prepare_jobs",
            Self::ExecuteJobs => "execute_jobs",
            Self::Postprocess => "postprocess",
        }
    }
}

struct ReviewPipelineDagContext<'a> {
    diff_content: &'a str,
    config: Option<config::Config>,
    repo_path: &'a Path,
    on_progress: Option<ProgressCallback>,
    verification_reuse_cache: crate::review::verification::VerificationReuseCache,
    services: Option<PipelineServices>,
    session: Option<ReviewSession>,
    prepared_jobs: Option<PreparedReviewJobs>,
    execution_summary: Option<ExecutionSummary>,
    review_result: Option<ReviewResult>,
}

impl<'a> ReviewPipelineDagContext<'a> {
    fn new(
        diff_content: &'a str,
        config: config::Config,
        repo_path: &'a Path,
        on_progress: Option<ProgressCallback>,
        verification_reuse_cache: crate::review::verification::VerificationReuseCache,
    ) -> Self {
        Self {
            diff_content,
            config: Some(config),
            repo_path,
            on_progress,
            verification_reuse_cache,
            services: None,
            session: None,
            prepared_jobs: None,
            execution_summary: None,
            review_result: None,
        }
    }

    fn into_result(self) -> Result<ReviewResult> {
        self.review_result
            .ok_or_else(|| anyhow::anyhow!("review DAG did not produce a review result"))
    }
}

pub(super) async fn execute_review_pipeline_dag(
    diff_content: &str,
    config: config::Config,
    repo_path: &Path,
    on_progress: Option<ProgressCallback>,
    verification_reuse_cache: crate::review::verification::VerificationReuseCache,
) -> Result<ReviewResult> {
    let specs = build_review_pipeline_specs();
    let dag_description = describe_dag(&specs);
    debug!(?dag_description, "Executing review pipeline DAG");
    let mut context = ReviewPipelineDagContext::new(
        diff_content,
        config,
        repo_path,
        on_progress,
        verification_reuse_cache,
    );
    let records = execute_dag(&specs, &mut context, |stage, context| {
        async move { execute_stage(stage, context).await }.boxed()
    })
    .await?;
    let mut result = context.into_result()?;
    result.dag_traces.insert(
        0,
        DagExecutionTrace {
            graph_name: "review_pipeline".to_string(),
            records,
        },
    );
    Ok(result)
}

fn stage_hints(stage: ReviewPipelineStage) -> DagNodeExecutionHints {
    match stage {
        ReviewPipelineStage::InitializeServices
        | ReviewPipelineStage::BuildSession
        | ReviewPipelineStage::PrepareJobs
        | ReviewPipelineStage::Postprocess => DagNodeExecutionHints {
            parallelizable: false,
            retryable: true,
            timeout_ms: None,
            side_effects: false,
            subgraph: match stage {
                ReviewPipelineStage::Postprocess => Some("review_postprocess".to_string()),
                _ => None,
            },
        },
        ReviewPipelineStage::ExecuteJobs => DagNodeExecutionHints {
            parallelizable: true,
            retryable: true,
            timeout_ms: None,
            side_effects: false,
            subgraph: None,
        },
    }
}

pub(in super::super) fn describe_review_pipeline_graph() -> DagGraphContract {
    DagGraphContract {
        name: "review_pipeline".to_string(),
        description:
            "Top-level review pipeline DAG from service initialization through postprocessing."
                .to_string(),
        entry_nodes: vec!["initialize_services".to_string()],
        terminal_nodes: vec!["postprocess".to_string()],
        nodes: vec![
            DagNodeContract {
                name: "initialize_services".to_string(),
                description: "Build adapters, rules, plugin manager, feedback stores, and shared pipeline services.".to_string(),
                kind: DagNodeKind::Setup,
                dependencies: vec![],
                inputs: vec!["config".to_string(), "repo_path".to_string()],
                outputs: vec!["pipeline_services".to_string()],
                hints: stage_hints(ReviewPipelineStage::InitializeServices),
                enabled: true,
            },
            DagNodeContract {
                name: "build_session".to_string(),
                description: "Parse diffs, build context indexes, enhanced guidance, and session-scoped state.".to_string(),
                kind: DagNodeKind::Preparation,
                dependencies: vec!["initialize_services".to_string()],
                inputs: vec!["diff_content".to_string(), "pipeline_services".to_string(), "progress_callback".to_string()],
                outputs: vec!["review_session".to_string()],
                hints: stage_hints(ReviewPipelineStage::BuildSession),
                enabled: true,
            },
            DagNodeContract {
                name: "prepare_jobs".to_string(),
                description: "Generate file/pass review jobs and deterministic analyzer comments.".to_string(),
                kind: DagNodeKind::Preparation,
                dependencies: vec!["build_session".to_string()],
                inputs: vec!["pipeline_services".to_string(), "review_session".to_string()],
                outputs: vec!["prepared_review_jobs".to_string()],
                hints: stage_hints(ReviewPipelineStage::PrepareJobs),
                enabled: true,
            },
            DagNodeContract {
                name: "execute_jobs".to_string(),
                description: "Run LLM and agent-backed review jobs and aggregate execution summaries.".to_string(),
                kind: DagNodeKind::Execution,
                dependencies: vec!["prepare_jobs".to_string()],
                inputs: vec!["prepared_review_jobs".to_string(), "pipeline_services".to_string(), "review_session".to_string()],
                outputs: vec!["execution_summary".to_string()],
                hints: stage_hints(ReviewPipelineStage::ExecuteJobs),
                enabled: true,
            },
            DagNodeContract {
                name: "postprocess".to_string(),
                description: "Apply verification, feedback adjustments, filters, and suppression to produce the final review result.".to_string(),
                kind: DagNodeKind::Transformation,
                dependencies: vec!["execute_jobs".to_string()],
                inputs: vec!["execution_summary".to_string(), "pipeline_services".to_string(), "review_session".to_string()],
                outputs: vec!["review_result".to_string()],
                hints: stage_hints(ReviewPipelineStage::Postprocess),
                enabled: true,
            },
        ],
    }
}

fn build_review_pipeline_specs() -> Vec<DagNodeSpec<ReviewPipelineStage>> {
    vec![
        DagNodeSpec {
            id: ReviewPipelineStage::InitializeServices,
            dependencies: vec![],
            hints: stage_hints(ReviewPipelineStage::InitializeServices),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPipelineStage::BuildSession,
            dependencies: vec![ReviewPipelineStage::InitializeServices],
            hints: stage_hints(ReviewPipelineStage::BuildSession),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPipelineStage::PrepareJobs,
            dependencies: vec![ReviewPipelineStage::BuildSession],
            hints: stage_hints(ReviewPipelineStage::PrepareJobs),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPipelineStage::ExecuteJobs,
            dependencies: vec![ReviewPipelineStage::PrepareJobs],
            hints: stage_hints(ReviewPipelineStage::ExecuteJobs),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPipelineStage::Postprocess,
            dependencies: vec![ReviewPipelineStage::ExecuteJobs],
            hints: stage_hints(ReviewPipelineStage::Postprocess),
            enabled: true,
        },
    ]
}

async fn execute_stage(
    stage: ReviewPipelineStage,
    context: &mut ReviewPipelineDagContext<'_>,
) -> Result<()> {
    match stage {
        ReviewPipelineStage::InitializeServices => execute_services_stage(context).await,
        ReviewPipelineStage::BuildSession => execute_session_stage(context).await,
        ReviewPipelineStage::PrepareJobs => execute_prepare_stage(context).await,
        ReviewPipelineStage::ExecuteJobs => execute_job_stage(context).await,
        ReviewPipelineStage::Postprocess => execute_postprocess_stage(context).await,
    }
}

async fn execute_services_stage(context: &mut ReviewPipelineDagContext<'_>) -> Result<()> {
    let config = context
        .config
        .take()
        .ok_or_else(|| anyhow::anyhow!("services stage missing config"))?;
    context.services = Some(PipelineServices::new(config, context.repo_path).await?);
    Ok(())
}

async fn execute_session_stage(context: &mut ReviewPipelineDagContext<'_>) -> Result<()> {
    let services = context
        .services
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("session stage missing services"))?;
    context.session = Some(
        ReviewSession::new(
            context.diff_content,
            services,
            context.on_progress.clone(),
            std::mem::take(&mut context.verification_reuse_cache),
        )
        .await?,
    );
    Ok(())
}

async fn execute_prepare_stage(context: &mut ReviewPipelineDagContext<'_>) -> Result<()> {
    let prepared_jobs = {
        let services = context
            .services
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("prepare stage missing services"))?;
        let session = context
            .session
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("prepare stage missing session"))?;
        prepare_file_review_jobs(services, session).await?
    };
    context.prepared_jobs = Some(prepared_jobs);
    Ok(())
}

async fn execute_job_stage(context: &mut ReviewPipelineDagContext<'_>) -> Result<()> {
    let prepared = context
        .prepared_jobs
        .take()
        .ok_or_else(|| anyhow::anyhow!("execution stage missing prepared jobs"))?;
    let execution_summary = {
        let services = context
            .services
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("execution stage missing services"))?;
        let session = context
            .session
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("execution stage missing session"))?;
        execute_review_jobs(
            prepared.jobs,
            ReviewExecutionContext {
                services,
                session,
                initial_comments: prepared.all_comments,
                files_completed: prepared.files_completed,
                files_skipped: prepared.files_skipped,
            },
        )
        .await?
    };
    context.execution_summary = Some(execution_summary);
    Ok(())
}

async fn execute_postprocess_stage(context: &mut ReviewPipelineDagContext<'_>) -> Result<()> {
    let execution_summary = context
        .execution_summary
        .take()
        .ok_or_else(|| anyhow::anyhow!("postprocess stage missing execution summary"))?;
    let review_result = {
        let services = context
            .services
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("postprocess stage missing services"))?;
        let session = context
            .session
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("postprocess stage missing session"))?;
        run_postprocess(execution_summary, services, session).await?
    };
    context.review_result = Some(review_result);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_review_pipeline_specs_are_linearized_as_a_dag() {
        let descriptions = describe_dag(&build_review_pipeline_specs());

        assert_eq!(descriptions[0].name, "initialize_services");
        assert_eq!(descriptions[1].dependencies, vec!["initialize_services"]);
        assert_eq!(descriptions[4].dependencies, vec!["execute_jobs"]);
    }

    #[test]
    fn review_pipeline_graph_contract_exposes_inputs_and_outputs() {
        let graph = describe_review_pipeline_graph();

        assert_eq!(graph.name, "review_pipeline");
        assert_eq!(graph.entry_nodes, vec!["initialize_services"]);
        assert_eq!(graph.terminal_nodes, vec!["postprocess"]);
        assert_eq!(
            graph.nodes[2].inputs,
            vec!["pipeline_services", "review_session"]
        );
        assert_eq!(graph.nodes[4].outputs, vec!["review_result"]);
        assert_eq!(
            graph.nodes[4].hints.subgraph.as_deref(),
            Some("review_postprocess")
        );
    }
}
