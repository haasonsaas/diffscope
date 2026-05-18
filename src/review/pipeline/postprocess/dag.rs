use anyhow::Result;
use futures::FutureExt;
use tracing::{debug, info};

#[path = "blast_radius.rs"]
mod blast_radius;
#[path = "context_sources.rs"]
mod context_sources;

use crate::core;
use crate::core::dag::{
    describe_dag, execute_dag, DagExecutionTrace, DagGraphContract, DagNode, DagNodeContract,
    DagNodeExecutionHints, DagNodeKind, DagNodeSpec,
};

use super::super::super::filters::{apply_feedback_confidence_adjustment, apply_review_filters};
use super::super::contracts::ExecutionSummary;
use super::super::repo_support::save_convention_store;
use super::super::services::PipelineServices;
use super::super::session::ReviewSession;
use super::super::types::{AgentActivity, FileMetric, ReviewResult};
use super::dedup::deduplicate_specialized_comments;
use super::feedback::apply_semantic_feedback_adjustment;
use super::suppression::apply_convention_suppression;
use super::verification::{apply_verification_pass, VerificationPassOutput};
use blast_radius::apply_blast_radius_summaries;
use context_sources::apply_context_source_tags;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ReviewPostprocessStage {
    SpecializedDedup,
    PluginPostProcessors,
    Verification,
    SemanticFeedback,
    FeedbackCalibration,
    ReviewFilters,
    EnhancedFilters,
    ConventionSuppression,
    BlastRadius,
    ContextSourceArtifacts,
    PrioritizeFindings,
    SaveConventionStore,
}

impl DagNode for ReviewPostprocessStage {
    fn name(&self) -> &'static str {
        match self {
            Self::SpecializedDedup => "specialized_dedup",
            Self::PluginPostProcessors => "plugin_postprocessors",
            Self::Verification => "verification",
            Self::SemanticFeedback => "semantic_feedback",
            Self::FeedbackCalibration => "feedback_calibration",
            Self::ReviewFilters => "review_filters",
            Self::EnhancedFilters => "enhanced_filters",
            Self::ConventionSuppression => "convention_suppression",
            Self::BlastRadius => "blast_radius",
            Self::ContextSourceArtifacts => "context_source_artifacts",
            Self::PrioritizeFindings => "prioritize_findings",
            Self::SaveConventionStore => "save_convention_store",
        }
    }
}

struct ReviewPostprocessDagContext<'a> {
    services: &'a PipelineServices,
    session: &'a mut ReviewSession,
    comments: Vec<core::Comment>,
    total_prompt_tokens: usize,
    total_completion_tokens: usize,
    total_tokens: usize,
    file_metrics: Vec<FileMetric>,
    comments_by_pass: std::collections::HashMap<String, usize>,
    agent_activity: Option<AgentActivity>,
    verification_output: Option<VerificationPassOutput>,
    convention_suppressed_count: usize,
}

impl<'a> ReviewPostprocessDagContext<'a> {
    fn new(
        execution: ExecutionSummary,
        services: &'a PipelineServices,
        session: &'a mut ReviewSession,
    ) -> Self {
        Self {
            services,
            session,
            comments: execution.all_comments,
            total_prompt_tokens: execution.total_prompt_tokens,
            total_completion_tokens: execution.total_completion_tokens,
            total_tokens: execution.total_tokens,
            file_metrics: execution.file_metrics,
            comments_by_pass: execution.comments_by_pass,
            agent_activity: execution.agent_activity,
            verification_output: None,
            convention_suppressed_count: 0,
        }
    }

    fn into_result(self) -> ReviewResult {
        let (verification_report, warnings) = self
            .verification_output
            .map(|output| (output.report, output.warnings))
            .unwrap_or((None, Vec::new()));

        ReviewResult {
            comments: self.comments,
            total_prompt_tokens: self.total_prompt_tokens,
            total_completion_tokens: self.total_completion_tokens,
            total_tokens: self.total_tokens,
            file_metrics: self.file_metrics,
            convention_suppressed_count: self.convention_suppressed_count,
            comments_by_pass: self.comments_by_pass,
            hotspots: self.session.enhanced_ctx.hotspots.clone(),
            agent_activity: self.agent_activity,
            verification_report,
            verification_reuse_cache: self.session.verification_reuse_cache.clone(),
            warnings,
            dag_traces: self.session.graph_query_traces.clone(),
        }
    }
}

pub(super) async fn run_postprocess_dag(
    execution: ExecutionSummary,
    services: &PipelineServices,
    session: &mut ReviewSession,
) -> Result<ReviewResult> {
    let specs = build_postprocess_specs(&services.config, services.convention_store_path.is_some());
    let dag_description = describe_dag(&specs);
    debug!(?dag_description, "Executing review postprocess DAG");
    let mut context = ReviewPostprocessDagContext::new(execution, services, session);
    let records = execute_dag(&specs, &mut context, |stage, context| {
        async move { execute_stage(stage, context).await }.boxed()
    })
    .await?;
    let mut result = context.into_result();
    result.dag_traces.push(DagExecutionTrace {
        graph_name: "review_postprocess".to_string(),
        records,
    });
    Ok(result)
}

fn stage_hints(stage: ReviewPostprocessStage) -> DagNodeExecutionHints {
    match stage {
        ReviewPostprocessStage::SaveConventionStore => DagNodeExecutionHints {
            parallelizable: false,
            retryable: true,
            timeout_ms: Some(10_000),
            side_effects: true,
            subgraph: None,
        },
        _ => DagNodeExecutionHints {
            parallelizable: false,
            retryable: true,
            timeout_ms: None,
            side_effects: false,
            subgraph: None,
        },
    }
}

pub(in super::super) fn describe_review_postprocess_graph(
    config: &crate::config::Config,
    has_convention_store_path: bool,
) -> DagGraphContract {
    let nodes = build_postprocess_specs(config, has_convention_store_path)
        .into_iter()
        .map(|spec| match spec.id {
            ReviewPostprocessStage::SpecializedDedup => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Deduplicate semantically overlapping comments on the same file and line before later transforms."
                        .to_string(),
                kind: DagNodeKind::Transformation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec!["comments".to_string()],
                outputs: vec!["comments".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::PluginPostProcessors => DagNodeContract {
                name: spec.id.name().to_string(),
                description: "Run plugin-defined postprocessors over accumulated review comments."
                    .to_string(),
                kind: DagNodeKind::Transformation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec!["comments".to_string(), "repo_path".to_string()],
                outputs: vec!["comments".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::Verification => DagNodeContract {
                name: spec.id.name().to_string(),
                description: "Run one or more verification judges and keep or filter LLM comments."
                    .to_string(),
                kind: DagNodeKind::Validation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec![
                    "comments".to_string(),
                    "verification_adapters".to_string(),
                    "review_session".to_string(),
                ],
                outputs: vec![
                    "comments".to_string(),
                    "verification_report".to_string(),
                    "warnings".to_string(),
                ],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::SemanticFeedback => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Adjust comment confidence using semantic feedback retrieval and embeddings."
                        .to_string(),
                kind: DagNodeKind::Analysis,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec![
                    "comments".to_string(),
                    "semantic_feedback_store".to_string(),
                    "embedding_adapter".to_string(),
                ],
                outputs: vec!["comments".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::FeedbackCalibration => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Adjust comment confidence from accumulated human feedback statistics."
                        .to_string(),
                kind: DagNodeKind::Transformation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec!["comments".to_string(), "feedback_store".to_string()],
                outputs: vec!["comments".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::ReviewFilters => DagNodeContract {
                name: spec.id.name().to_string(),
                description: "Apply generic review filtering rules such as confidence thresholds."
                    .to_string(),
                kind: DagNodeKind::Transformation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec![
                    "comments".to_string(),
                    "config".to_string(),
                    "feedback_store".to_string(),
                ],
                outputs: vec!["comments".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::EnhancedFilters => DagNodeContract {
                name: spec.id.name().to_string(),
                description: "Apply enhanced filter pipeline stages backed by convention learning."
                    .to_string(),
                kind: DagNodeKind::Transformation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec![
                    "comments".to_string(),
                    "enhanced_review_context".to_string(),
                ],
                outputs: vec!["comments".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::ConventionSuppression => DagNodeContract {
                name: spec.id.name().to_string(),
                description: "Suppress comments that match learned team convention patterns."
                    .to_string(),
                kind: DagNodeKind::Transformation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec!["comments".to_string(), "convention_store".to_string()],
                outputs: vec![
                    "comments".to_string(),
                    "convention_suppressed_count".to_string(),
                ],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::BlastRadius => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Annotate findings with cross-file blast-radius summaries from reverse dependency counts."
                        .to_string(),
                kind: DagNodeKind::Transformation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec!["comments".to_string(), "symbol_index".to_string()],
                outputs: vec!["comments".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::ContextSourceArtifacts => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Attach stable context-source tags that identify which external or supplemental context informed each file review."
                        .to_string(),
                kind: DagNodeKind::Transformation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec!["comments".to_string(), "verification_context".to_string()],
                outputs: vec!["comments".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::PrioritizeFindings => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Re-rank final findings using the calibrated confidence and category priority."
                        .to_string(),
                kind: DagNodeKind::Transformation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec!["comments".to_string()],
                outputs: vec!["comments".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            ReviewPostprocessStage::SaveConventionStore => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Persist convention-learning state after the postprocess pipeline completes."
                        .to_string(),
                kind: DagNodeKind::Persistence,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec![
                    "convention_store".to_string(),
                    "convention_store_path".to_string(),
                ],
                outputs: vec!["convention_store_saved".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
        })
        .collect::<Vec<_>>();

    DagGraphContract {
        name: "review_postprocess".to_string(),
        description:
            "Granular postprocess DAG that transforms raw execution comments into a filtered review result."
                .to_string(),
        entry_nodes: vec!["specialized_dedup".to_string()],
        terminal_nodes: vec![if has_convention_store_path {
            "save_convention_store".to_string()
        } else {
            "prioritize_findings".to_string()
        }],
        nodes,
    }
}

fn build_postprocess_specs(
    config: &crate::config::Config,
    has_convention_store_path: bool,
) -> Vec<DagNodeSpec<ReviewPostprocessStage>> {
    vec![
        DagNodeSpec {
            id: ReviewPostprocessStage::SpecializedDedup,
            dependencies: vec![],
            hints: stage_hints(ReviewPostprocessStage::SpecializedDedup),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::PluginPostProcessors,
            dependencies: vec![ReviewPostprocessStage::SpecializedDedup],
            hints: stage_hints(ReviewPostprocessStage::PluginPostProcessors),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::Verification,
            dependencies: vec![ReviewPostprocessStage::PluginPostProcessors],
            hints: stage_hints(ReviewPostprocessStage::Verification),
            enabled: config.verification.enabled,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::SemanticFeedback,
            dependencies: vec![ReviewPostprocessStage::Verification],
            hints: stage_hints(ReviewPostprocessStage::SemanticFeedback),
            enabled: config.semantic_feedback,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::FeedbackCalibration,
            dependencies: vec![ReviewPostprocessStage::SemanticFeedback],
            hints: stage_hints(ReviewPostprocessStage::FeedbackCalibration),
            enabled: config.enhanced_feedback,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::ReviewFilters,
            dependencies: vec![ReviewPostprocessStage::FeedbackCalibration],
            hints: stage_hints(ReviewPostprocessStage::ReviewFilters),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::EnhancedFilters,
            dependencies: vec![ReviewPostprocessStage::ReviewFilters],
            hints: stage_hints(ReviewPostprocessStage::EnhancedFilters),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::ConventionSuppression,
            dependencies: vec![ReviewPostprocessStage::EnhancedFilters],
            hints: stage_hints(ReviewPostprocessStage::ConventionSuppression),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::BlastRadius,
            dependencies: vec![ReviewPostprocessStage::ConventionSuppression],
            hints: stage_hints(ReviewPostprocessStage::BlastRadius),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::ContextSourceArtifacts,
            dependencies: vec![ReviewPostprocessStage::BlastRadius],
            hints: stage_hints(ReviewPostprocessStage::ContextSourceArtifacts),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::PrioritizeFindings,
            dependencies: vec![ReviewPostprocessStage::ContextSourceArtifacts],
            hints: stage_hints(ReviewPostprocessStage::PrioritizeFindings),
            enabled: true,
        },
        DagNodeSpec {
            id: ReviewPostprocessStage::SaveConventionStore,
            dependencies: vec![ReviewPostprocessStage::PrioritizeFindings],
            hints: stage_hints(ReviewPostprocessStage::SaveConventionStore),
            enabled: has_convention_store_path,
        },
    ]
}

async fn execute_stage(
    stage: ReviewPostprocessStage,
    context: &mut ReviewPostprocessDagContext<'_>,
) -> Result<()> {
    match stage {
        ReviewPostprocessStage::SpecializedDedup => execute_specialized_dedup_stage(context),
        ReviewPostprocessStage::PluginPostProcessors => {
            execute_plugin_postprocessors_stage(context).await
        }
        ReviewPostprocessStage::Verification => execute_verification_stage(context).await,
        ReviewPostprocessStage::SemanticFeedback => execute_semantic_feedback_stage(context).await,
        ReviewPostprocessStage::FeedbackCalibration => execute_feedback_calibration_stage(context),
        ReviewPostprocessStage::ReviewFilters => execute_review_filters_stage(context),
        ReviewPostprocessStage::EnhancedFilters => execute_enhanced_filters_stage(context),
        ReviewPostprocessStage::ConventionSuppression => {
            execute_convention_suppression_stage(context)
        }
        ReviewPostprocessStage::BlastRadius => execute_blast_radius_stage(context),
        ReviewPostprocessStage::ContextSourceArtifacts => {
            execute_context_source_artifacts_stage(context)
        }
        ReviewPostprocessStage::PrioritizeFindings => execute_prioritize_findings_stage(context),
        ReviewPostprocessStage::SaveConventionStore => execute_convention_store_save_stage(context),
    }
}

fn execute_specialized_dedup_stage(context: &mut ReviewPostprocessDagContext<'_>) -> Result<()> {
    let before = context.comments.len();
    let comments = std::mem::take(&mut context.comments);
    context.comments = deduplicate_specialized_comments(comments);
    let after = context.comments.len();
    if before != after {
        info!(
            "Deduplicated {} semantically overlapping comment(s) ({} -> {})",
            before - after,
            before,
            after
        );
    }
    Ok(())
}

async fn execute_plugin_postprocessors_stage(
    context: &mut ReviewPostprocessDagContext<'_>,
) -> Result<()> {
    let repo_path_str = context.services.repo_path_str();
    let comments = std::mem::take(&mut context.comments);
    context.comments = context
        .services
        .plugin_manager
        .run_post_processors(comments, &repo_path_str)
        .await?;
    Ok(())
}

async fn execute_verification_stage(context: &mut ReviewPostprocessDagContext<'_>) -> Result<()> {
    let comments = std::mem::take(&mut context.comments);
    let output = apply_verification_pass(comments, context.services, context.session).await;
    context.comments = output.comments.clone();
    context.verification_output = Some(output);
    Ok(())
}

async fn execute_semantic_feedback_stage(
    context: &mut ReviewPostprocessDagContext<'_>,
) -> Result<()> {
    let comments = std::mem::take(&mut context.comments);
    context.comments = apply_semantic_feedback_adjustment(
        comments,
        context.session.semantic_feedback_store.as_ref(),
        context.services.embedding_adapter.as_deref(),
        &context.services.config,
    )
    .await;
    Ok(())
}

fn execute_feedback_calibration_stage(context: &mut ReviewPostprocessDagContext<'_>) -> Result<()> {
    let comments = std::mem::take(&mut context.comments);
    context.comments = apply_feedback_confidence_adjustment(
        comments,
        &context.services.feedback,
        context.services.config.feedback_min_observations,
    );
    Ok(())
}

fn execute_review_filters_stage(context: &mut ReviewPostprocessDagContext<'_>) -> Result<()> {
    let comments = std::mem::take(&mut context.comments);
    context.comments = apply_review_filters(
        comments,
        &context.services.config,
        &context.services.feedback,
    );
    Ok(())
}

fn execute_enhanced_filters_stage(context: &mut ReviewPostprocessDagContext<'_>) -> Result<()> {
    let comments = std::mem::take(&mut context.comments);
    context.comments = core::apply_enhanced_filters(&mut context.session.enhanced_ctx, comments);
    Ok(())
}

fn execute_convention_suppression_stage(
    context: &mut ReviewPostprocessDagContext<'_>,
) -> Result<()> {
    let comments = std::mem::take(&mut context.comments);
    let (comments, suppressed_count) =
        apply_convention_suppression(comments, &context.session.enhanced_ctx.convention_store);
    context.comments = comments;
    context.convention_suppressed_count = suppressed_count;
    Ok(())
}

fn execute_blast_radius_stage(context: &mut ReviewPostprocessDagContext<'_>) -> Result<()> {
    let comments = std::mem::take(&mut context.comments);
    context.comments =
        apply_blast_radius_summaries(comments, context.session.symbol_index.as_ref());
    Ok(())
}

fn execute_context_source_artifacts_stage(
    context: &mut ReviewPostprocessDagContext<'_>,
) -> Result<()> {
    let comments = std::mem::take(&mut context.comments);
    context.comments = apply_context_source_tags(comments, &context.session.verification_context);
    Ok(())
}

fn execute_prioritize_findings_stage(context: &mut ReviewPostprocessDagContext<'_>) -> Result<()> {
    core::sort_comments_by_priority(&mut context.comments);
    Ok(())
}

fn execute_convention_store_save_stage(
    context: &mut ReviewPostprocessDagContext<'_>,
) -> Result<()> {
    if let Some(ref store_path) = context.services.convention_store_path {
        save_convention_store(&context.session.enhanced_ctx.convention_store, store_path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_postprocess_specs_exposes_granular_nodes() {
        let descriptions = describe_dag(&build_postprocess_specs(
            &crate::config::Config::default(),
            false,
        ));

        assert_eq!(descriptions[0].name, "specialized_dedup");
        assert_eq!(descriptions[3].name, "semantic_feedback");
        assert_eq!(
            descriptions
                .last()
                .map(|description| description.dependencies.clone())
                .unwrap(),
            vec!["prioritize_findings"]
        );
        assert!(descriptions
            .iter()
            .any(|description| description.name == "blast_radius"));
        assert!(descriptions
            .iter()
            .any(|description| description.name == "context_source_artifacts"));
        assert!(descriptions
            .iter()
            .any(|description| description.name == "prioritize_findings"));
    }

    #[test]
    fn review_postprocess_graph_contract_exposes_verification_and_persistence() {
        let graph = describe_review_postprocess_graph(&crate::config::Config::default(), true);

        assert_eq!(graph.name, "review_postprocess");
        assert_eq!(graph.entry_nodes, vec!["specialized_dedup"]);
        assert!(graph.nodes.iter().any(|node| node.name == "verification"
            && node.outputs.contains(&"verification_report".to_string())));
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.name == "save_convention_store" && node.hints.side_effects));
        assert_eq!(graph.terminal_nodes, vec!["save_convention_store"]);
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.name == "context_source_artifacts"
                && node.inputs.contains(&"verification_context".to_string())));
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.name == "prioritize_findings"));
    }
}
